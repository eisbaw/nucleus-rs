#![no_std]
#![no_main]

// Minimal Cortex-M7 firmware: emit a known sentinel over USART1, then
// spin. Proves the M10 path end-to-end — a no_std bin (cortex-m-rt
// vector table + entry + panic handler) whose USART1 output Renode
// captures to a file backend. The embedded-pattern backend's generated
// firmware (TASK-0048) will follow this shape; here the "computation"
// is just a constant sentinel.

use core::panic::PanicInfo;
use cortex_m_rt::entry;

// STM32H7 USART1 — Renode models it as UART.STM32F7_USART @ 0x4001_1000
// (see platforms/cpus/stm32h743.repl). STM32F7/H7 USART register layout:
//   CR1 @ +0x00  (UE = bit 0, TE = bit 3)
//   ISR @ +0x1C  (TXE = bit 7: transmit data register empty)
//   TDR @ +0x28  (transmit data register)
const USART1_CR1: *mut u32 = 0x4001_1000 as *mut u32;
const USART1_ISR: *const u32 = 0x4001_101C as *const u32;
const USART1_TDR: *mut u32 = 0x4001_1028 as *mut u32;

const TXE: u32 = 1 << 7;

fn putc(b: u8) {
    unsafe {
        while core::ptr::read_volatile(USART1_ISR) & TXE == 0 {}
        core::ptr::write_volatile(USART1_TDR, b as u32);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable the USART (UE) and its transmitter (TE).
        core::ptr::write_volatile(USART1_CR1, (1 << 0) | (1 << 3));
    }
    for &b in b"NUCLEUS-M10-OK\n" {
        putc(b);
    }
    loop {}
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}
