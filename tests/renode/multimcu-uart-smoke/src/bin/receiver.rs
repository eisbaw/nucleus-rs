#![no_std]
#![no_main]

// M11 inter-MCU de-risk — RECEIVER MCU (TASK-0049.01). Runs as Renode
// machine "receiver". Its USART1 is wired to the same UARTHub as the
// sender's USART1, so bytes the sender transmits arrive in this MCU's
// USART1 receive queue (RXNE). It relays every received byte out USART2,
// which the .resc captures to a file backend. A captured sentinel proves
// the bytes crossed the inter-MCU hub AND this MCU received + acted on
// them — i.e. wired MCU-to-MCU transport works end-to-end in Renode.
//
// The relay loop is unbounded (Renode's RunFor bounds wall time); the
// UARTBase receive queue buffers, so ordering between the two machines is
// forgiving — bytes sent before this loop first polls are still queued.

use core::panic::PanicInfo;
use cortex_m_rt::entry;
use multimcu_uart_smoke::{cr1_write, putc, try_getc, CR1_RE, CR1_TE, CR1_UE, USART1_BASE, USART2_BASE};

#[entry]
fn main() -> ! {
    unsafe {
        // USART1: enable + receiver (read from the hub).
        cr1_write(USART1_BASE, CR1_UE | CR1_RE);
        // USART2: enable + transmitter (relay out; captured by the .resc).
        cr1_write(USART2_BASE, CR1_UE | CR1_TE);
        loop {
            if let Some(b) = try_getc(USART1_BASE) {
                putc(USART2_BASE, b);
            }
        }
    }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}
