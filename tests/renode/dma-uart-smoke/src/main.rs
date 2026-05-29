#![no_std]
#![no_main]

// M10 AC#1 DE-RISK smoke (TASK-0048.11). Sibling of
// tests/renode/uart-smoke/src/main.rs, but the bytes leave the chip via
// the DMA ENGINE (a DMA1 MemoryToPeripheral transfer into USART1's TDR),
// NOT a polled CPU store loop. If Renode captures the payload, then the
// bundled stm32h743 model drives DMA-to-peripheral USART1 TX end-to-end,
// which is the gating prerequisite for the real async STM32H7 DMA shim
// (parent TASK-0048 AC#1). This is a PROOF harness only — it does not
// touch the production embedded-pattern Usart1Shim.

use core::panic::PanicInfo;
use cortex_m_rt::entry;

// STM32H7 USART1 — Renode models it as UART.STM32F7_USART @ 0x4001_1000
// (platforms/cpus/stm32h743.repl). Register layout (STM32F7_USART.cs):
//   CR1 @ +0x00  (UE = bit 0, TE = bit 3)
//   CR3 @ +0x08  (DMAT = bit 7: DMA enable transmitter)
//   TDR @ +0x28  (transmit data register — DMA writes land here)
const USART1_CR1: *mut u32 = 0x4001_1000 as *mut u32;
const USART1_CR3: *mut u32 = 0x4001_1008 as *mut u32;
const USART1_TDR: u32 = 0x4001_1028;

// STM32H7 DMA1 — Renode models it as DMA.STM32DMA @ 0x4002_0000
// (platforms/cpus/stm32h743.repl). Per-stream register layout (STM32DMA.cs
// `Registers` enum + StreamStep = 0x18 per stream); we use stream 0, so
// streamOffset = 0:
//   SxCR    @ base + 0x10   stream configuration
//   SxNDTR  @ base + 0x14   number of data items to transfer
//   SxPAR   @ base + 0x18   peripheral address (DMA destination for M->P)
//   SxM0AR  @ base + 0x1C   memory-0 address (DMA source for M->P)
const DMA1_S0CR: *mut u32 = 0x4002_0010 as *mut u32;
const DMA1_S0NDTR: *mut u32 = 0x4002_0014 as *mut u32;
const DMA1_S0PAR: *mut u32 = 0x4002_0018 as *mut u32;
const DMA1_S0M0AR: *mut u32 = 0x4002_001C as *mut u32;

// SxCR bit fields (STM32DMA.cs Stream::DefineRegisters):
//   EN   bit 0          stream enable (write 1 => HandleEnable)
//   DIR  bits 6..7      00=P->M, 01=M->P, 10=M->M
//   PINC bit 9          peripheral-address increment (0: fixed = TDR)
//   MINC bit 10         memory-address increment (1: walk the buffer)
//   PSIZE bits 11..12   00 = byte
//   MSIZE bits 13..14   00 = byte
const DMA_DIR_MEM_TO_PERIPH: u32 = 0b01 << 6;
const DMA_MINC: u32 = 1 << 10;
const DMA_EN: u32 = 1 << 0;

// Payload lives in a RAM (.data) buffer so the DMA source is a RAM region,
// mirroring "DMA the computed output array out" (the real shim's job).
// Renode reads DTCM fine; real silicon would need AXI SRAM (see memory.x).
const PAYLOAD: &[u8] = b"NUC-DMA-OK\n";
static mut TX_BUF: [u8; 11] = [0u8; 11];

#[entry]
fn main() -> ! {
    unsafe {
        // 1. Enable the USART + its transmitter. CR1 = UE|TE is load-bearing:
        //    the model drops the byte if the transmitter is not enabled
        //    (HandleTransmitData checks transmitEnabled && enabled). DMAT
        //    (CR3 bit 7) gates the TX-DMA request on real silicon; under
        //    Renode it is a no-op flag (no callback / no TX request line in
        //    the platform) but we set it for hardware faithfulness.
        core::ptr::write_volatile(USART1_CR1, (1 << 0) | (1 << 3));
        core::ptr::write_volatile(USART1_CR3, 1 << 7);

        // 2. Fill the RAM source buffer with the payload.
        let buf = core::ptr::addr_of_mut!(TX_BUF);
        for (i, &b) in PAYLOAD.iter().enumerate() {
            (*buf)[i] = b;
        }

        // 3. Program the DMA stream: source = RAM buffer (incrementing),
        //    destination = USART1 TDR (fixed). PSIZE/MSIZE both byte (00).
        core::ptr::write_volatile(DMA1_S0PAR, USART1_TDR);
        core::ptr::write_volatile(DMA1_S0M0AR, buf as u32);
        core::ptr::write_volatile(DMA1_S0NDTR, PAYLOAD.len() as u32);

        // 4. Configure direction/increment with EN still 0, THEN set EN in a
        //    second write. Renode's STM32DMA HandleEnable reads direction at
        //    the moment EN is written; configuring it first guarantees the
        //    MemoryToPeripheral immediate-transfer path is taken (not the
        //    PeripheralToMemory request-driven path). This is also the real
        //    STM32 HAL ordering (configure, then enable last).
        core::ptr::write_volatile(DMA1_S0CR, DMA_DIR_MEM_TO_PERIPH | DMA_MINC);
        core::ptr::write_volatile(DMA1_S0CR, DMA_DIR_MEM_TO_PERIPH | DMA_MINC | DMA_EN);
    }

    // The DMA performs the whole transfer synchronously on the EN write in
    // Renode, so by here the bytes are already in the USART file backend.
    loop {}
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}
