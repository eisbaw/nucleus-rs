#![no_std]

//! Shared no_std USART register helpers for the M11 inter-MCU de-risk smoke
//! (TASK-0049.01). Kept in one place so the sender and receiver bins cannot
//! drift on register layout (silent-sibling defence). Renode models USART1/2
//! as UART.STM32F7_USART (platforms/cpus/stm32h743.repl):
//!   USART1 @ 0x4001_1000, USART2 @ 0x4000_4400.
//! Register layout (STM32F7_USART.cs):
//!   CR1 @ +0x00  (UE bit0, RE bit2, TE bit3)
//!   ISR @ +0x1C  (RXNE bit5: receive data register not empty; TXE bit7)
//!   RDR @ +0x24  (receive data register; reading dequeues one byte)
//!   TDR @ +0x28  (transmit data register)

pub const USART1_BASE: usize = 0x4001_1000;
pub const USART2_BASE: usize = 0x4000_4400;

const CR1: usize = 0x00;
const ISR: usize = 0x1C;
const RDR: usize = 0x24;
const TDR: usize = 0x28;

const TXE: u32 = 1 << 7;
const RXNE: u32 = 1 << 5;

/// CR1 enable bits. UE is load-bearing (the model gates both TX and RX on
/// `enabled`); TE gates transmit, RE gates receive.
pub const CR1_UE: u32 = 1 << 0;
pub const CR1_RE: u32 = 1 << 2;
pub const CR1_TE: u32 = 1 << 3;

/// Write CR1 (enable the USART + the direction(s) needed).
///
/// # Safety
/// `base` must be a valid memory-mapped USART register block.
pub unsafe fn cr1_write(base: usize, val: u32) {
    core::ptr::write_volatile((base + CR1) as *mut u32, val);
}

/// Blocking transmit of one byte (poll TXE, then write TDR). Under Renode
/// TXE is hardwired true so this never actually waits, but it is the
/// correct pattern and harmless.
///
/// # Safety
/// `base` must be a valid memory-mapped USART register block with TE set.
pub unsafe fn putc(base: usize, b: u8) {
    while core::ptr::read_volatile((base + ISR) as *const u32) & TXE == 0 {}
    core::ptr::write_volatile((base + TDR) as *mut u32, b as u32);
}

/// Non-blocking receive: returns Some(byte) if RXNE is set (a byte arrived
/// over the wire / hub), else None. Reading RDR dequeues the byte.
///
/// # Safety
/// `base` must be a valid memory-mapped USART register block with RE set.
pub unsafe fn try_getc(base: usize) -> Option<u8> {
    if core::ptr::read_volatile((base + ISR) as *const u32) & RXNE != 0 {
        Some(core::ptr::read_volatile((base + RDR) as *const u32) as u8)
    } else {
        None
    }
}
