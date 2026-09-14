// SPDX-License-Identifier: Apache-2.0
//! The router: the bus's one request channel fanned out to a channel
//! per device by address, and the devices' responses merged back into
//! one. Bits 15 to 12 of the address choose: one is the data memory,
//! two the timer, three the serial port, and any other address is a
//! hole, which the router answers itself with zero for a read and
//! swallows for a write. A request is taken only when the channel it
//! goes to has room, a receive under a condition; responses come one
//! at a time, since the core waits for each load's answer before it
//! makes another request, and the memory's is taken before the
//! timer's and the timer's before the port's if two were ever
//! offered.
use crate::bus::REQ_ADDR;
use txhdl::comp::{mux, Clock, DefaultClock, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

#[derive(Trace, Default)]
pub struct Router {}

#[lower]
impl
    Unit<
        (Rx<U<69>>, Rx<U<32>>, Rx<U<32>>, Rx<U<32>>),
        (Tx<U<69>>, Tx<U<69>>, Tx<U<69>>, Tx<U<32>>),
    > for Router
{
    async fn run(
        &mut self,
        (req, mem_resp, timer_resp, uart_resp): (
            Rx<U<69>>,
            Rx<U<32>>,
            Rx<U<32>>,
            Rx<U<32>>,
        ),
        (mem_req, timer_req, uart_req, resp): (
            Tx<U<69>>,
            Tx<U<69>>,
            Tx<U<69>>,
            Tx<U<32>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            // Where the request at the head would go, and whether that
            // channel has room; the head is looked at before it is
            // taken.
            let offered = req.peek().is_some();
            let head = req.head();
            let region = head.slice::<REQ_ADDR, 32>().slice::<12, 4>();
            let to_mem = region == 1;
            let to_timer = region == 2;
            let to_uart = region == 3;
            let hole = !to_mem & !to_timer & !to_uart;
            let we = head.bit(0);
            let room = mux(
                to_mem,
                mem_req.ready(),
                mux(
                    to_timer,
                    timer_req.ready(),
                    mux(
                        to_uart,
                        uart_req.ready(),
                        mux(we, Bit::One, resp.ready()),
                    ),
                ),
            );
            let go = offered & room;
            let r = req.recv_if(room).unwrap_or_default();
            when!(go & to_mem => { mem_req.send(r) });
            when!(go & to_timer => { timer_req.send(r) });
            when!(go & to_uart => { uart_req.send(r) });
            // The responses, merged: the memory's first, then the timer's.
            let m_valid = mem_resp.peek().is_some();
            let m = mem_resp.recv().unwrap_or_default();
            let t_valid = timer_resp.peek().is_some() & !m_valid;
            let t = timer_resp.recv_if(!m_valid).unwrap_or_default();
            let u_valid = uart_resp.peek().is_some() & !m_valid & !t_valid;
            let u = uart_resp.recv_if(!m_valid & !t_valid).unwrap_or_default();
            let hole_read = go & hole & !we;
            let answer = mux(
                m_valid,
                m,
                mux(t_valid, t, mux(u_valid, u, U::<32>::from(0u32))),
            );
            when!(m_valid | t_valid | u_valid | hole_read => {
                resp.send(answer)
            });
        }
    }
}
