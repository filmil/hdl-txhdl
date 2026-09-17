// SPDX-License-Identifier: Apache-2.0
//! Video behind AXI-Lite, for an HDMI encoder chip. A host client on an
//! AXI4 link paints a framebuffer through the AXI-Lite bridge, and the
//! video peripheral shows it: a raster of colour, syncs and a data
//! enable, as the SiI9134 on the AX7A200B takes them. Beside it, the
//! I2C master configures the chip, and a model of an I2C device on the
//! same lines records what it was sent.
//!
//! The mode is a small one, 16 by 12 visible with each framebuffer
//! pixel 2 by 2 on the screen, so the framebuffer is 8 by 6 and a frame
//! is 408 cycles. The client sets the cursor to the corner and paints
//! all 48 pixels in one fixed burst to the pixel word, then waits for
//! two frames. The run rebuilds the last whole frame from the pixels the
//! data enable marks and checks every one of them against what was
//! painted. The peripheral and the I2C master are lowered, and the build
//! simulates both netlists against this run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, BurstKind, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::hdmi::{Hdmi, I2cDevice, I2cInit, SII9134_WRITES};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

/// The mode: 16 visible columns, 2 of front porch, 3 of sync and 3 of
/// back porch; 12 visible rows, 1, 2 and 2; a framebuffer pixel 2 by 2.
type Video = Hdmi<16, 2, 3, 3, 12, 1, 2, 2, 1>;
const COLS: usize = 8;
const ROWS: usize = 6;

/// The I2C master: a quarter bit of four cycles, and a reset of ten.
type Master = I2cInit<4, 10>;

/// The peripheral's words.
const STATUS: u32 = 0x1000;
const CURSOR: u32 = 0x1004;
const PIXEL: u32 = 0x1008;

/// The colour painted at a framebuffer pixel: red across, green down,
/// blue on the diagonal.
fn colour(x: usize, y: usize) -> u32 {
    let r = (x * 2) as u32;
    let g = (y * 3) as u32;
    let b = if x == y { 15 } else { 0 };
    r << 8 | g << 4 | b
}

/// A 12-bit colour as the 24 bits the chip is given.
fn wide(c: u32) -> u32 {
    let w = |n: u32| n << 4 | n;
    w(c >> 8 & 15) << 16 | w(c >> 4 & 15) << 8 | w(c & 15)
}

fn write_at(addr: u32, burst: BurstKind) -> Wr<32> {
    let mut wr = Wr::at(addr);
    wr.size = U::from(2u8);
    wr.burst = burst;
    wr
}

fn main() {
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<32, 32, 4, 2, 4>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    let lite = axi_lite::<32, 32, 4>();
    let (law, lar, lw, lb, lr) = lite.host;
    let (paw, par, pw, pb, pr) = lite.per;
    let (rgb_out, rgb) = signal::<U<24>, DefaultClock>();
    let (hs_out, hsync) = signal::<Bit, DefaultClock>();
    let (vs_out, vsync) = signal::<Bit, DefaultClock>();
    let (de_out, de) = signal::<Bit, DefaultClock>();
    let (sda_line, sda_in) = signal::<Bit, DefaultClock>();
    let (rst_out, nreset) = signal::<Bit, DefaultClock>();
    let (scl_out, scl_low) = signal::<Bit, DefaultClock>();
    let (sdal_out, sda_low) = signal::<Bit, DefaultClock>();
    let (done_out, done) = signal::<Bit, DefaultClock>();
    let (fail_out, failed) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut video = Video::default();
    let mut master = Master::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("aw", &paw);
        wave.add("ar", &par);
        wave.add("w", &pw);
        wave.add("b", &pb);
        wave.add("r", &pr);
        wave.add("rgb", &rgb);
        wave.add("hsync", &hsync);
        wave.add("vsync", &vsync);
        wave.add("de", &de);
        wave.add("video", &video);
        wave.add("sda_in", &sda_in);
        wave.add("nreset", &nreset);
        wave.add("scl_low", &scl_low);
        wave.add("sda_low", &sda_low);
        wave.add("done", &done);
        wave.add("failed", &failed);
        wave.add("master", &master);
        wave.start();
    }

    let client = async move {
        let corner = [U::from(0u32)];
        let wr = host.write(write_at(CURSOR, BurstKind::Incr), &corner).await;
        assert_eq!(wr.done().await.resp, Resp::Okay);
        let pixels: Vec<U<32>> = (0..ROWS)
            .flat_map(|y| (0..COLS).map(move |x| U::from(colour(x, y))))
            .collect();
        let wr = host.write(write_at(PIXEL, BurstKind::Fixed), &pixels).await;
        assert_eq!(wr.done().await.resp, Resp::Okay);
        println!("t={:>5} painted {} pixels", now(), pixels.len());
        // Wait until two more frames have been shown.
        let frames = |w: u128| (w >> 16) as u32;
        let first = frames(
            host.read(Rd::at(STATUS, 1)).await.done().await.data[0].raw(),
        );
        loop {
            let st = host.read(Rd::at(STATUS, 1)).await.done().await;
            if frames(st.data[0].raw()) >= first + 2 {
                println!("t={:>5} status {:#x}", now(), st.data[0].raw());
                break;
            }
        }
    };

    let hardware = join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
        ),
        join2(
            video
                .run((paw, par, pw), (pb, pr, rgb_out, hs_out, vs_out, de_out)),
            master
                .run(sda_in, (rst_out, scl_out, sdal_out, done_out, fail_out)),
        ),
    );
    let mut sim = Running::new(join2(hardware, client));
    let mut device = I2cDevice::new();
    // The picture, rebuilt frame by frame from the pixels the enable
    // marks, a frame starting at the vertical sync.
    let mut frame: Vec<u32> = Vec::new();
    let mut last_whole: Vec<u32> = Vec::new();
    let mut was_sync = false;
    for _ in 0..1200 {
        sim.cycle();
        device.step(scl_low.get().to_bool(), sda_low.get().to_bool());
        let line = !sda_low.get().to_bool() && !device.pulls_sda();
        sda_line.set(Bit::from_bool(line));
        let sync = !vsync.get().to_bool();
        if sync && !was_sync {
            if frame.len() == 4 * COLS * ROWS {
                last_whole = std::mem::take(&mut frame);
            }
            frame.clear();
        }
        was_sync = sync;
        if de.get().to_bool() {
            frame.push(rgb.get().raw() as u32);
        }
    }
    stop();
    println!("the chip was sent:");
    for t in &device.transactions {
        println!("  {:02x?}", t);
    }
    let want: Vec<Vec<u8>> = SII9134_WRITES
        .iter()
        .map(|&(d, r, v)| vec![d, r, v])
        .collect();
    assert_eq!(device.transactions, want, "the chip's configuration");
    assert!(done.get().to_bool() && !failed.get().to_bool());
    println!("the last whole frame, a framebuffer pixel per entry:");
    let width = 2 * COLS;
    for y in 0..ROWS {
        let row: Vec<String> = (0..COLS)
            .map(|x| format!("{:06x}", last_whole[2 * y * width + 2 * x]))
            .collect();
        println!("  {}", row.join(" "));
    }
    for (i, px) in last_whole.iter().enumerate() {
        let (sx, sy) = (i % width, i / width);
        let want = wide(colour(sx / 2, sy / 2));
        assert_eq!(*px, want, "screen pixel {sx},{sy}");
    }
    println!("every screen pixel is the colour painted");
    let video_net = Video::lowered("hdmi_video");
    let master_net = Master::lowered("hdmi_i2c");
    txhdl::netlist::write_netlists_from_env(&[&video_net, &master_net]);
}
