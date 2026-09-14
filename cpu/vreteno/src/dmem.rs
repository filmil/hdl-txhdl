// SPDX-License-Identifier: Apache-2.0
//! The data memory as a device on the bus: 1024 words at `DATA_BASE`
//! in four memories of a byte, one per lane, so that a store of a
//! byte or a half writes its lanes and reads nothing. A write lands in
//! the lanes the request covers; a read goes into the memory's own
//! register at the edge, the synchronous read a block RAM has, and is
//! answered the cycle after. The router sends this device only the
//! requests in its range, so it checks no address.
use crate::bus::{REQ_ADDR, REQ_WDATA};
use txhdl::comp::{Clock, DefaultClock, Mem, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

/// Words of data memory.
pub const DMEM_WORDS: usize = 1024;

#[derive(Trace, Default)]
pub struct Dmem {
    pub lane0: Mem<U<8>, DMEM_WORDS>,
    pub lane1: Mem<U<8>, DMEM_WORDS>,
    pub lane2: Mem<U<8>, DMEM_WORDS>,
    pub lane3: Mem<U<8>, DMEM_WORDS>,
    /// The word a read landed in, and whether it is to be answered.
    pub word: Reg<U<32>>,
    pub answer: Reg<Bit>,
}

impl Dmem {
    /// A word of the memory, for the run and the test to look at.
    pub fn data_word(&self, at: usize) -> u32 {
        let lane = |m: &Mem<U<8>, DMEM_WORDS>| m.read(at).raw() as u32;
        lane(&self.lane0)
            | lane(&self.lane1) << 8
            | lane(&self.lane2) << 16
            | lane(&self.lane3) << 24
    }
}

#[lower]
impl Unit for Dmem {
    async fn run(&mut self, req: Rx<U<69>>, resp: Tx<U<32>>) {
        loop {
            DefaultClock::rising().await;
            let (offered, r) = req.take();
            let at = r.slice::<REQ_ADDR, 32>().slice::<2, 10>();
            let data = r.slice::<REQ_WDATA, 32>();
            let we = r.bit(0);
            let write = offered & we;
            when!(write & r.bit(1) => {
                self.lane0.at(at) <= data.slice::<0, 8>()
            });
            when!(write & r.bit(2) => {
                self.lane1.at(at) <= data.slice::<8, 8>()
            });
            when!(write & r.bit(3) => {
                self.lane2.at(at) <= data.slice::<16, 8>()
            });
            when!(write & r.bit(4) => {
                self.lane3.at(at) <= data.slice::<24, 8>()
            });
            let read = offered & !we;
            when!(read => {
                self.word <= self
                    .lane3
                    .read(at)
                    .concat::<_, 16>(self.lane2.read(at))
                    .concat::<_, 24>(self.lane1.read(at))
                    .concat::<_, 32>(self.lane0.read(at));
                self.answer <= Bit::One
            } else {
                self.answer <= Bit::Zero
            });
            when!(self.answer => { resp.send(self.word) });
        }
    }
}
