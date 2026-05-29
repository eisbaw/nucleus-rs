/* STM32H743 memory map (from Renode platforms/cpus/stm32h743.repl) —
   identical to tests/renode/uart-smoke/memory.x:
   flashBank1 @ 0x08000000 (size 0x100000 = 1024K) and DTCM @ 0x20000000
   (size 0x20000 = 128K). FLASH here is a strict subset (128K of 1024K).
   RAM here is the FULL DTCM (128K == 0x20000) — do NOT raise RAM LENGTH
   past 128K without mapping a larger region (e.g. axiSram @ 0x24000000),
   or the stack would silently overflow DTCM with no linker error.

   NOTE for the real-DMA shim (TASK-0048 AC#1): the DMA source buffer here
   is a `static` linked into RAM (DTCM). Renode models DTCM as a plain
   sysbus MappedMemory, so the DMA engine reads it fine — but on REAL
   STM32H7 silicon DTCM is NOT reachable by the DMA1/DMA2 controllers
   (only AXI SRAM / SRAM1-3 are). A real-DMA dma_push must therefore place
   the source buffer in a DMA-accessible region; Renode papers over this. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
