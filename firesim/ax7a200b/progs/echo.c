// SPDX-License-Identifier: Apache-2.0
// The UART's input: every byte read is sent back, until a 'q'.
#include "uart.h"
int main(void) {
    uart_init();
    puts_("echo: type, q ends\n");
    for (;;) {
        int c = getc_();
        if (c < 0) continue;
        if (c == 'q') break;
        putc_((char)c);
    }
    puts_("\necho: done\n");
    return 0;
}
