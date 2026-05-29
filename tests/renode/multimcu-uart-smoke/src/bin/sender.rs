#![no_std]
#![no_main]

// M11 inter-MCU de-risk — SENDER MCU (TASK-0049.01). Runs as Renode machine
// "sender". Transmits a sentinel over USART1, which is wired to a UARTHub
// shared with the receiver MCU. Pure TX (same shape as uart-smoke); the
// cross-MCU delivery + receive is the receiver bin's job.

use core::panic::PanicInfo;
use cortex_m_rt::entry;
use multimcu_uart_smoke::{cr1_write, putc, CR1_TE, CR1_UE, USART1_BASE};

#[entry]
fn main() -> ! {
    unsafe {
        cr1_write(USART1_BASE, CR1_UE | CR1_TE);
        for &b in b"M11-LINK-OK\n" {
            putc(USART1_BASE, b);
        }
    }
    loop {}
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}
