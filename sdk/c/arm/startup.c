/* Bare-metal startup for running the libadip test suite on an ARM target.
 *
 * Test scaffolding, not part of the library. It exists so that the same test
 * sources that run on a host can run on the kind of core an appliance uses.
 * That is the only way to find out whether the fixed-point arithmetic behaves
 * identically on a 32-bit core, and every range check in the library rests on
 * that arithmetic being exact.
 *
 * A manufacturer does not use this file. Their vendor's startup code already
 * does these jobs before main runs.
 *
 * newlib's own crt0 is deliberately not used. It asks the debug monitor for
 * heap and stack limits through semihosting, and under QEMU that call returns
 * nothing it can use, after which it hangs before reaching main. Supplying the
 * handful of stubs below is both smaller and entirely under our control.
 */

#include <stddef.h>
#include <stdint.h>

extern unsigned _etext, __data_start__, __data_end__, __bss_start__, __bss_end__;
extern unsigned _stack_top, __end__;
extern int main(void);

void reset_handler(void);

/* ------------------------------------------------------------- semihosting
 *
 * The program traps with a known breakpoint and the emulator performs the
 * operation on its behalf. This is how output reaches the terminal while the
 * code runs on the emulated core.
 */

#define SYS_WRITEC       0x03
#define SYS_EXIT         0x18
#define SYS_EXIT_EXTENDED 0x20

#define STOPPED_APPLICATION_EXIT 0x20026u

/* r1 carries a value for some operations and a pointer for others, so it is
 * taken as a word and cast at each call site. */
static int semihost(int op, uint32_t arg)
{
    register int r0 __asm__("r0") = op;
    register uint32_t r1 __asm__("r1") = arg;
    __asm__ volatile("bkpt 0xAB" : "+r"(r0) : "r"(r1) : "memory");
    return r0;
}

/* A fault stops here rather than running on with the core in an undefined
 * state. A product enters its safe state instead; see the failsafe in
 * adip_runtime.c. */
static void hang(void)
{
    for (;;) {
    }
}

/* The core loads the stack pointer from slot 0 and jumps to slot 1 on reset.
 * The remaining entries are present so that a fault lands somewhere known
 * instead of executing whatever follows in flash.
 *
 * Held as addresses rather than as function pointers because the first entry
 * is a stack address, not code, and a table typed as function pointers
 * requires converting an object pointer to a function pointer, which ISO C
 * does not define.
 */
__attribute__((section(".vectors"), used))
static const uintptr_t vectors[] = {
    (uintptr_t)&_stack_top,
    (uintptr_t)&reset_handler,
    (uintptr_t)&hang,              /* NMI */
    (uintptr_t)&hang,              /* HardFault */
    (uintptr_t)&hang,              /* MemManage */
    (uintptr_t)&hang,              /* BusFault */
    (uintptr_t)&hang,              /* UsageFault */
};

void reset_handler(void)
{
    unsigned *src = &_etext;
    unsigned *dst;
    int status;

    /* Initialised data is held in flash and copied to RAM. libadip declares
     * none, which is the point of generating its tables as const, but the C
     * library has its own. */
    for (dst = &__data_start__; dst < &__data_end__; ) {
        *dst++ = *src++;
    }
    for (dst = &__bss_start__; dst < &__bss_end__; ) {
        *dst++ = 0;
    }

    /* No call to __libc_init_array. It runs C++ constructors and the
     * constructors registered by the C library's own startup files, and with
     * -nostartfiles neither exists. The tests are plain C. */
    status = main();

    /* Report the test result so a build script can tell pass from fail without
     * reading the output.
     *
     * On 32-bit ARM, plain SYS_EXIT takes the reason code in r1 as a value and
     * carries no exit status; the two-word block form belongs to AArch64.
     * Passing a block here made every run report failure regardless of the
     * result. SYS_EXIT_EXTENDED is the 32-bit operation that does carry a
     * status, with plain SYS_EXIT kept for a monitor that lacks it.
     */
    {
        uint32_t block[2];
        block[0] = STOPPED_APPLICATION_EXIT;
        block[1] = (uint32_t)status;
        (void)semihost(SYS_EXIT_EXTENDED, (uint32_t)(uintptr_t)block);
        (void)semihost(SYS_EXIT, STOPPED_APPLICATION_EXIT);
    }
    hang();
}

/* ------------------------------------------------------- newlib's syscalls
 *
 * printf needs somewhere to put characters and somewhere to allocate its
 * buffer. Nothing else here is reached by the tests, and each stub fails
 * rather than pretending to succeed.
 */

int _write(int file, const char *data, int len);
int _write(int file, const char *data, int len)
{
    int i;
    (void)file;
    for (i = 0; i < len; i++) {
        char c = data[i];
        (void)semihost(SYS_WRITEC, (uint32_t)(uintptr_t)&c);
    }
    return len;
}

void *_sbrk(ptrdiff_t increment);
void *_sbrk(ptrdiff_t increment)
{
    static char *brk = NULL;
    char *previous;

    if (brk == NULL) {
        brk = (char *)&__end__;
    }
    previous = brk;
    brk += increment;
    return previous;
}

int _close(int file);
int _close(int file)
{
    (void)file;
    return -1;
}

int _lseek(int file, int offset, int whence);
int _lseek(int file, int offset, int whence)
{
    (void)file;
    (void)offset;
    (void)whence;
    return -1;
}

int _read(int file, char *data, int len);
int _read(int file, char *data, int len)
{
    (void)file;
    (void)data;
    (void)len;
    return -1;
}

int _isatty(int file);
int _isatty(int file)
{
    (void)file;
    return 1;
}

/* Declared with the shape newlib expects. The tests never stat anything, so
 * reporting a character device is enough to keep stdout unbuffered-friendly. */
int _fstat(int file, void *st);
int _fstat(int file, void *st)
{
    (void)file;
    (void)st;
    return -1;
}

void _exit(int status);
void _exit(int status)
{
    uint32_t block[2];
    block[0] = STOPPED_APPLICATION_EXIT;
    block[1] = (uint32_t)status;
    (void)semihost(SYS_EXIT_EXTENDED, (uint32_t)(uintptr_t)block);
    (void)semihost(SYS_EXIT, STOPPED_APPLICATION_EXIT);
    hang();
}
