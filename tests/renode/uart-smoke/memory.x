/* STM32H743 memory map (from Renode platforms/cpus/stm32h743.repl):
   flashBank1 @ 0x08000000, DTCM @ 0x20000000. Lengths trimmed to a
   safe subset for this tiny firmware. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
