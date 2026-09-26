/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * The Vreteno Ethernet port.
 *
 * Two slots each way, ping-pong, with a whole frame in each. The
 * registers are ten words:
 *
 *   0x00  read:  which slot the received frame is in.
 *   0x04  read:  how many bytes are in it.
 *   0x08  r/w1c: bit 0, a frame has arrived. Written back to clear.
 *   0x0c  r/w:   bit 0, let a received frame raise the interrupt.
 *   0x10  write: which slot to send from.
 *   0x14  write: how many bytes to send.
 *   0x18  write: 1 to send it.
 *   0x1c  read:  bit 0, the transmitter will take a frame.
 *   0x20  r/w1c: bit 0, a frame went out. Not used; see below.
 *   0x24  r/w:   bit 0, let a sent frame raise the interrupt.
 *
 * `lib/parts/src/eth.rs` is the hardware and states the same map;
 * `cpu/vreteno/tests/zephyr_dts.rs` checks that this file and that one
 * still agree.
 *
 * There is no transmit interrupt path here, and the hardware's
 * transmit event is left disabled. Zephyr's `send` may block, so the
 * driver polls `tx_ready` and sleeps between tries, which is what the
 * in-tree driver for the closest comparable hardware does and it
 * saves an interrupt the design would otherwise have to route.
 *
 * Frames are copied rather than handed over. A `net_pkt` is a chain of
 * fragments and `net_pkt_read` and `net_pkt_write` both linearise, so
 * the hardware only ever sees one contiguous buffer and a byte count.
 * That is why there is no scatter-gather here and none in the
 * peripheral.
 */

#define DT_DRV_COMPAT hdlfactory_vreteno_eth

#define LOG_MODULE_NAME eth_vreteno
#define LOG_LEVEL CONFIG_ETHERNET_LOG_LEVEL

#include <zephyr/logging/log.h>
LOG_MODULE_REGISTER(LOG_MODULE_NAME);

#include <zephyr/device.h>
#include <zephyr/init.h>
#include <zephyr/irq.h>
#include <zephyr/kernel.h>
#include <zephyr/net/ethernet.h>
#include <zephyr/net/net_if.h>
#include <zephyr/net/net_pkt.h>
/*
 * `sys_read32` and `sys_write32` are the architecture's, not the
 * generic header's, exactly as in `uart_vreteno.c`. Including only
 * `zephyr/sys/sys_io.h` compiles to an implicit declaration.
 */
#include <zephyr/arch/cpu.h>
#include <zephyr/sys/sys_io.h>
#include <vreteno/regs/ethslots.h>

#define VRETENO_ETH_RX_SLOT       ETHSLOTS_RX_SLOT
#define VRETENO_ETH_RX_LENGTH     ETHSLOTS_RX_LENGTH
#define VRETENO_ETH_RX_EV_PENDING ETHSLOTS_RX_EV_PENDING
#define VRETENO_ETH_RX_EV_ENABLE  ETHSLOTS_RX_EV_ENABLE
#define VRETENO_ETH_TX_SLOT       ETHSLOTS_TX_SLOT
#define VRETENO_ETH_TX_LENGTH     ETHSLOTS_TX_LENGTH
#define VRETENO_ETH_TX_START      ETHSLOTS_TX_START
#define VRETENO_ETH_TX_READY      ETHSLOTS_TX_READY
#define VRETENO_ETH_TX_EV_PENDING ETHSLOTS_TX_EV_PENDING
#define VRETENO_ETH_TX_EV_ENABLE  ETHSLOTS_TX_EV_ENABLE

#define VRETENO_ETH_EVENT ETHSLOTS_RX_EV_PENDING_PENDING_MASK

/* A whole frame, which is `FRAME_MAX` in the hardware. */
#define VRETENO_ETH_SLOT_SIZE 2048
#define VRETENO_ETH_SLOTS     2

/*
 * How long `send` will wait for the transmitter, in milliseconds a
 * try. A frame of 1500 bytes takes 12 microseconds on the wire at a
 * gigabit, so a transmitter that is not ready within this many
 * milliseconds is not busy, it is broken.
 */
#define VRETENO_ETH_TX_TRIES 100

struct eth_vreteno_data {
	struct net_if *iface;
	uint8_t mac_addr[6];
	uint8_t tx_slot;
};

struct eth_vreteno_config {
	mem_addr_t base;
	uint8_t *tx_buf[VRETENO_ETH_SLOTS];
	uint8_t *rx_buf[VRETENO_ETH_SLOTS];
	void (*config_func)(const struct device *dev);
};

static inline uint32_t eth_vreteno_read(const struct device *dev,
					mem_addr_t reg)
{
	const struct eth_vreteno_config *cfg = dev->config;

	return sys_read32(cfg->base + reg);
}

static inline void eth_vreteno_write(const struct device *dev,
				     mem_addr_t reg, uint32_t value)
{
	const struct eth_vreteno_config *cfg = dev->config;

	sys_write32(value, cfg->base + reg);
}

static int eth_vreteno_send(const struct device *dev, struct net_pkt *pkt)
{
	struct eth_vreteno_data *data = dev->data;
	const struct eth_vreteno_config *cfg = dev->config;
	uint16_t len = net_pkt_get_len(pkt);
	int tries = 0;

	if (len > VRETENO_ETH_SLOT_SIZE) {
		LOG_ERR("frame of %u bytes is larger than a slot", len);
		return -EMSGSIZE;
	}

	/*
	 * Out of the fragment chain and into one buffer. The byte count
	 * that goes to the hardware is this one and it is never rounded
	 * up: a frame padded to a word is a frame with bytes in it that
	 * nobody sent.
	 */
	if (net_pkt_read(pkt, cfg->tx_buf[data->tx_slot], len) < 0) {
		LOG_ERR("could not read the frame out of its fragments");
		return -EIO;
	}

	/*
	 * The frame has to be in memory before the hardware fetches it,
	 * and on this machine saying so takes a read.
	 *
	 * Stores are posted: the core does not wait for one to land, and
	 * issue 420 was a handler that wrote a register and returned
	 * before the write arrived. `fence` does not help, because this
	 * core decodes it as a known instruction and then does nothing
	 * with it (`Fence => {}` in `cpu/vreteno/src/model.rs`). The
	 * buffers are in DDR3 rather than inside the peripheral, so the
	 * frame's bytes and the `tx_start` that starts the fetch travel
	 * to two different places on the bus and nothing orders them.
	 *
	 * Reading the last byte back stalls the core until memory
	 * answers, which is the only barrier software has here. Without
	 * it a frame goes out carrying whatever was in the slot before,
	 * intermittently and only under load, which is the worst shape a
	 * fault can have.
	 *
	 * Be clear about what this rests on, because it is not the
	 * architecture. AXI does not order a read against a write: they
	 * are separate channels, and `AxiHost` turns the core's single
	 * issue stream into an address phase on one or the other, after
	 * which keeping them in order is the memory's business rather
	 * than the bus's.
	 *
	 * What makes it work here is measured rather than promised:
	 * `a_load_behind_an_unanswered_store_to_one_address` in
	 * `lib/parts/src/bus/axi/sim.rs` issues a store, does not await
	 * its response, and loads the same address. The load is answered
	 * at tick 44 and the store's response arrives at 46, so the
	 * store was genuinely outstanding, and the load returned the
	 * stored value anyway. The test asserts that ordering as well as
	 * the value, so it fails rather than going quietly vacuous if
	 * the store ever starts completing first.
	 *
	 * So this is correct for our link, and it would stop being
	 * correct if the interconnect changed. Issue 432 is the gap
	 * underneath it: with `fence` inert and stores posted, there is
	 * no architectural way to say this, only a measured one.
	 */
	{
		volatile const uint8_t *last =
			&cfg->tx_buf[data->tx_slot][len - 1];
		(void)*last;
	}

	while ((eth_vreteno_read(dev, VRETENO_ETH_TX_READY) &
		VRETENO_ETH_EVENT) == 0) {
		if (tries++ == VRETENO_ETH_TX_TRIES) {
			LOG_ERR("the transmitter never became ready");
			return -ETIMEDOUT;
		}
		k_sleep(K_MSEC(1));
	}

	eth_vreteno_write(dev, VRETENO_ETH_TX_SLOT, data->tx_slot);
	eth_vreteno_write(dev, VRETENO_ETH_TX_LENGTH, len);
	eth_vreteno_write(dev, VRETENO_ETH_TX_START, 1);

	data->tx_slot = (data->tx_slot + 1) % VRETENO_ETH_SLOTS;

	return 0;
}

static void eth_vreteno_receive(const struct device *dev)
{
	struct eth_vreteno_data *data = dev->data;
	const struct eth_vreteno_config *cfg = dev->config;
	struct net_pkt *pkt;
	uint16_t len;
	uint8_t slot;

	if (!net_if_flag_is_set(data->iface, NET_IF_UP)) {
		return;
	}

	len = eth_vreteno_read(dev, VRETENO_ETH_RX_LENGTH);
	slot = eth_vreteno_read(dev, VRETENO_ETH_RX_SLOT) %
	       VRETENO_ETH_SLOTS;

	if (len == 0 || len > VRETENO_ETH_SLOT_SIZE) {
		LOG_ERR("a frame of %u bytes, which cannot be one", len);
		return;
	}

	pkt = net_pkt_rx_alloc_with_buffer(data->iface, len, AF_UNSPEC, 0,
					   K_NO_WAIT);
	if (pkt == NULL) {
		LOG_ERR("no buffer for a frame of %u bytes", len);
		return;
	}

	if (net_pkt_write(pkt, cfg->rx_buf[slot], len) < 0) {
		LOG_ERR("could not write the frame into its fragments");
		net_pkt_unref(pkt);
		return;
	}

	if (net_recv_data(data->iface, pkt) < 0) {
		LOG_ERR("the stack would not take the frame");
		net_pkt_unref(pkt);
	}
}

static void eth_vreteno_isr(const struct device *dev)
{
	uint32_t pending = eth_vreteno_read(dev, VRETENO_ETH_RX_EV_PENDING);

	if ((pending & VRETENO_ETH_EVENT) != 0) {
		eth_vreteno_receive(dev);
		/* Written back is how it clears. */
		eth_vreteno_write(dev, VRETENO_ETH_RX_EV_PENDING,
				  VRETENO_ETH_EVENT);
	}
}

static int eth_vreteno_start(const struct device *dev)
{
	/* Anything left pending from before is not ours. */
	eth_vreteno_write(dev, VRETENO_ETH_RX_EV_PENDING, VRETENO_ETH_EVENT);
	eth_vreteno_write(dev, VRETENO_ETH_TX_EV_PENDING, VRETENO_ETH_EVENT);
	eth_vreteno_write(dev, VRETENO_ETH_RX_EV_ENABLE, VRETENO_ETH_EVENT);

	return 0;
}

static int eth_vreteno_stop(const struct device *dev)
{
	eth_vreteno_write(dev, VRETENO_ETH_RX_EV_ENABLE, 0);

	return 0;
}

static enum ethernet_hw_caps eth_vreteno_caps(const struct device *dev)
{
	ARG_UNUSED(dev);

	/*
	 * The board's PHY is a gigabit part and the MAC's halves run on
	 * its clocks, so what the link negotiated is a property of the
	 * wire rather than of anything this driver sets.
	 */
	return ETHERNET_LINK_10BASE_T | ETHERNET_LINK_100BASE_T |
	       ETHERNET_LINK_1000BASE_T;
}

static int eth_vreteno_set_config(const struct device *dev,
				  enum ethernet_config_type type,
				  const struct ethernet_config *config)
{
	struct eth_vreteno_data *data = dev->data;

	if (type != ETHERNET_CONFIG_TYPE_MAC_ADDRESS) {
		return -ENOTSUP;
	}

	memcpy(data->mac_addr, config->mac_address.addr,
	       sizeof(data->mac_addr));

	return net_if_set_link_addr(data->iface, data->mac_addr,
				    sizeof(data->mac_addr),
				    NET_LINK_ETHERNET);
}

static void eth_vreteno_iface_init(struct net_if *iface)
{
	const struct device *dev = net_if_get_device(iface);
	const struct eth_vreteno_config *cfg = dev->config;
	struct eth_vreteno_data *data = dev->data;

	if (data->iface == NULL) {
		data->iface = iface;
	}

	ethernet_init(iface);

	if (net_if_set_link_addr(iface, data->mac_addr,
				 sizeof(data->mac_addr),
				 NET_LINK_ETHERNET) < 0) {
		LOG_ERR("the MAC address would not set");
		return;
	}

	cfg->config_func(dev);

	/*
	 * The carrier is declared on rather than learned. This machine's
	 * RGMII wrapper is hand-written and there is no Zephyr PHY driver
	 * behind it, so nothing here can be told that the cable came out;
	 * the link's state is visible on the board and not on the bus.
	 *
	 * What that costs: a stack which believes the carrier is up when
	 * it is not, and frames dropped into a dead wire rather than
	 * refused. When the PHY grows a driver, this becomes a
	 * `phy_link_callback_set` and `net_eth_carrier_on` moves into it.
	 */
	net_eth_carrier_on(iface);
}

static const struct ethernet_api eth_vreteno_api = {
	.iface_api.init = eth_vreteno_iface_init,
	.get_capabilities = eth_vreteno_caps,
	.send = eth_vreteno_send,
	.set_config = eth_vreteno_set_config,
	.start = eth_vreteno_start,
	.stop = eth_vreteno_stop,
};

static int eth_vreteno_init(const struct device *dev)
{
	/* Quiet until the interface comes up. */
	eth_vreteno_write(dev, VRETENO_ETH_RX_EV_ENABLE, 0);
	eth_vreteno_write(dev, VRETENO_ETH_TX_EV_ENABLE, 0);

	return 0;
}

/*
 * Receive and transmit are separate regions, and are bound separately
 * rather than as one region of four slots. One region would let a
 * transmit slot be computed at the address of a receive slot, which
 * is a frame arriving on top of one waiting to go out: it compiles,
 * it simulates whenever a test drives one direction at a time, and it
 * shows up as corruption under load. The hardware had exactly that
 * before the two regions were split apart.
 */
#define VRETENO_ETH_RX_BUFFERS(n) DT_INST_REG_ADDR_BY_NAME(n, rx_buffers)
#define VRETENO_ETH_TX_BUFFERS(n) DT_INST_REG_ADDR_BY_NAME(n, tx_buffers)

#define VRETENO_ETH_INIT(n)                                                \
	static void eth_vreteno_irq_config##n(const struct device *dev)    \
	{                                                                  \
		IRQ_CONNECT(DT_INST_IRQN(n), DT_INST_IRQ(n, priority),      \
			    eth_vreteno_isr, DEVICE_DT_INST_GET(n), 0);    \
		irq_enable(DT_INST_IRQN(n));                               \
	}                                                                  \
                                                                           \
	static struct eth_vreteno_data eth_vreteno_data##n = {             \
		.mac_addr = DT_INST_PROP(n, local_mac_address),            \
	};                                                                 \
                                                                           \
	static const struct eth_vreteno_config eth_vreteno_config##n = {   \
		.base = DT_INST_REG_ADDR_BY_NAME(n, registers),            \
		.config_func = eth_vreteno_irq_config##n,                  \
		.rx_buf = {                                                \
			(uint8_t *)VRETENO_ETH_RX_BUFFERS(n),              \
			(uint8_t *)(VRETENO_ETH_RX_BUFFERS(n) +            \
				    VRETENO_ETH_SLOT_SIZE),                \
		},                                                         \
		.tx_buf = {                                                \
			(uint8_t *)VRETENO_ETH_TX_BUFFERS(n),              \
			(uint8_t *)(VRETENO_ETH_TX_BUFFERS(n) +            \
				    VRETENO_ETH_SLOT_SIZE),                \
		},                                                         \
	};                                                                 \
                                                                           \
	ETH_NET_DEVICE_DT_INST_DEFINE(n, eth_vreteno_init, NULL,           \
				      &eth_vreteno_data##n,                \
				      &eth_vreteno_config##n,              \
				      CONFIG_ETH_INIT_PRIORITY,            \
				      &eth_vreteno_api, NET_ETH_MTU);

DT_INST_FOREACH_STATUS_OKAY(VRETENO_ETH_INIT)
