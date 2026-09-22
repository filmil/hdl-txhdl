/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * The Vreteno serial port.
 *
 * Three registers, which is the whole of the hardware:
 *
 *   0x00  write: the byte to send. Writing while busy drops the byte,
 *         so a caller polls STATUS_BUSY first.
 *   0x04  read: the status. Bit 0 is high while a byte is going out,
 *         bit 1 while a received byte is waiting, bit 2 while the
 *         receive buffer of eight is full.
 *   0x08  read: the oldest byte received, which reading consumes.
 *
 * `cpu/vreteno/src/uart.rs` is the hardware and states the same map;
 * `cpu/vreteno/tests/zephyr_dts.rs` checks that this file and that one
 * still agree, because a driver that is wrong by four bytes is a
 * machine that says nothing and gives no reason.
 *
 * There is no transmit interrupt and no baud rate register. The
 * divider is fixed at synthesis, `DIV` in the design, so
 * `current-speed` in the device tree is a statement of what the
 * hardware was built for rather than something this driver sets.
 */

#define DT_DRV_COMPAT hdlfactory_vreteno_uart

#include <zephyr/device.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/init.h>
/*
 * `sys_read32` and `sys_write32` are the architecture's, not the
 * generic header's: `zephyr/sys/sys_io.h` declares the port and bit
 * helpers, and the memory-mapped pair arrives through
 * `zephyr/arch/riscv/arch.h`, which this reaches. Including only the
 * generic header compiles to an implicit declaration and a driver
 * that reads the wrong width.
 */
#include <zephyr/arch/cpu.h>
#include <zephyr/sys/sys_io.h>

#define VRETENO_UART_DATA   0x00
#define VRETENO_UART_STATUS 0x04
#define VRETENO_UART_RX     0x08

#define VRETENO_STATUS_BUSY BIT(0)
#define VRETENO_STATUS_RX   BIT(1)
#define VRETENO_STATUS_FULL BIT(2)

struct uart_vreteno_config {
	uintptr_t base;
};

static uint32_t uart_vreteno_status(const struct device *dev)
{
	const struct uart_vreteno_config *cfg = dev->config;

	return sys_read32(cfg->base + VRETENO_UART_STATUS);
}

static int uart_vreteno_poll_in(const struct device *dev, unsigned char *c)
{
	const struct uart_vreteno_config *cfg = dev->config;

	if ((uart_vreteno_status(dev) & VRETENO_STATUS_RX) == 0U) {
		return -1;
	}

	*c = (unsigned char)sys_read32(cfg->base + VRETENO_UART_RX);
	return 0;
}

static void uart_vreteno_poll_out(const struct device *dev, unsigned char c)
{
	const struct uart_vreteno_config *cfg = dev->config;

	/* A byte written while the port is busy is dropped by the
	 * hardware rather than queued, so the wait is the interface.
	 */
	while ((uart_vreteno_status(dev) & VRETENO_STATUS_BUSY) != 0U) {
	}

	sys_write32((uint32_t)c, cfg->base + VRETENO_UART_DATA);
}

static int uart_vreteno_err_check(const struct device *dev)
{
	/* The one error the hardware reports is a receive buffer that
	 * was full when a byte arrived, which loses that byte. The
	 * design counts those; the count is not on the bus, so what
	 * this can say is that the buffer is full now.
	 */
	if ((uart_vreteno_status(dev) & VRETENO_STATUS_FULL) != 0U) {
		return UART_ERROR_OVERRUN;
	}

	return 0;
}

static int uart_vreteno_init(const struct device *dev)
{
	/* Nothing to configure: the divider is fixed at synthesis and
	 * the lines rest high out of reset.
	 */
	ARG_UNUSED(dev);
	return 0;
}

static DEVICE_API(uart, uart_vreteno_driver_api) = {
	.poll_in = uart_vreteno_poll_in,
	.poll_out = uart_vreteno_poll_out,
	.err_check = uart_vreteno_err_check,
};

#define UART_VRETENO_INIT(n)						\
	static const struct uart_vreteno_config uart_vreteno_config_##n = { \
		.base = DT_INST_REG_ADDR(n),				\
	};								\
									\
	DEVICE_DT_INST_DEFINE(n,					\
			      uart_vreteno_init,			\
			      NULL,					\
			      NULL,					\
			      &uart_vreteno_config_##n,			\
			      PRE_KERNEL_1,				\
			      CONFIG_SERIAL_INIT_PRIORITY,		\
			      &uart_vreteno_driver_api);

DT_INST_FOREACH_STATUS_OKAY(UART_VRETENO_INIT)
