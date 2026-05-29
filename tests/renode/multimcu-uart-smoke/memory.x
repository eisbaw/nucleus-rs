/* STM32H743 memory map (from Renode platforms/cpus/stm32h743.repl) —
   identical to tests/renode/uart-smoke/memory.x; both co-simulated
   machines load the same platform:
   flashBank1 @ 0x08000000 (1024K) and DTCM @ 0x20000000 (128K). FLASH
   here is a strict subset (128K of 1024K). RAM is the FULL DTCM (128K) —
   do NOT raise RAM LENGTH past 128K without mapping a larger region. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
