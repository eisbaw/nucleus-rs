/* STM32H743 memory map (from Renode platforms/cpus/stm32h743.repl):
   flashBank1 @ 0x08000000 (size 0x100000 = 1024K) and DTCM @ 0x20000000
   (size 0x20000 = 128K). FLASH here is a strict subset (128K of 1024K).
   RAM here is the FULL DTCM (128K == 0x20000) — NOT a trimmed subset, so
   do NOT raise RAM LENGTH past 128K without mapping a larger region
   (e.g. axiSram @ 0x24000000), or the stack would silently overflow DTCM
   with no linker error. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
