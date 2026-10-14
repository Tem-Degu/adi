/* Memory map of the Stellaris LM3S6965, which QEMU emulates as a Cortex-M3
 * board. Used only to run the libadip test suite on an ARM target. Nothing in
 * the library depends on this part; a manufacturer supplies their own script
 * and their vendor's startup code already does these jobs.
 */
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 256K
  SRAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 64K
}

ENTRY(reset_handler)

SECTIONS
{
  .text : {
    KEEP(*(.vectors))       /* first, because the core reads it at address 0 */
    *(.text*)
    *(.rodata*)

    /* newlib's constructor machinery. These were overlapping .data before,
     * because leaving them unplaced let the linker put them after _etext,
     * which is where the load address of .data starts. */
    KEEP(*(.init))
    KEEP(*(.fini))
    . = ALIGN(4);
    KEEP(*(.preinit_array*))
    KEEP(*(.init_array*))
    KEEP(*(.fini_array*))

    /* ARM unwind tables. Kept rather than discarded so the linker does not
     * place them after _etext, which is where .data loads from. C code never
     * unwinds, so they cost flash and nothing else. */
    . = ALIGN(4);
    *(.ARM.extab*)
    __exidx_start = .;
    *(.ARM.exidx*)
    __exidx_end = .;
    *(.eh_frame*)
    *(.gcc_except_table*)

    . = ALIGN(4);
    _etext = .;
  } > FLASH

  .data : AT (_etext) {
    . = ALIGN(4);
    __data_start__ = .;
    *(.data*)
    . = ALIGN(4);
    __data_end__ = .;
  } > SRAM

  .bss : {
    . = ALIGN(4);
    /* Names newlib's crt0 looks for. It zeroes this range itself, so the
     * startup file below does not. */
    __bss_start__ = .;
    *(.bss*)
    *(COMMON)
    . = ALIGN(4);
    __bss_end__ = .;
  } > SRAM

  .heap : {
    . = ALIGN(8);
    __end__ = .;
    PROVIDE(end = .);
    PROVIDE(_end = .);
    . = . + 0x4000;        /* printf buffers through newlib's allocator */
  } > SRAM

  _stack_top = ORIGIN(SRAM) + LENGTH(SRAM);
}
