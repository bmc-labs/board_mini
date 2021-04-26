/* Linker script for the STM32F103CBT6 */
MEMORY
{
  /* FLASH : ORIGIN = 0x08002000, LENGTH = 120K */ /* when using Maple BL */
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K  /* when using programmer */
  RAM : ORIGIN = 0x20000000, LENGTH = 20K
}
