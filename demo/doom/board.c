/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Doom on the board (issue 1177): doomgeneric's platform for Vreteno,
 * bare metal.
 *
 * * The screen. doomgeneric turns its 320 by 200 into 32-bit colour in a
 *   buffer of its own, at that size, and each frame is drawn from it
 *   into Razboj's framebuffer at 0x4200_0000, rows of 1024 words, at
 *   twice the size, every pixel four, forty rows down so that it sits in
 *   the middle of the 480; the scanout shows that buffer. Scaling here
 *   rather than in the game reads each pixel once, and a read of the
 *   DDR3 is what costs.
 * * The keys come over the serial port. A byte is a key pressed, and
 *   the key is let go a while after its last byte, so a terminal's
 *   repeat holds it down: the arrows, as a terminal sends them, or
 *   `w`, `a`, `s` and `d`; `f` fires, the space opens and uses, Enter,
 *   Escape, `y`, `n` and the digits are themselves.
 * * The time is the cycle counter, at the core's 100 MHz.
 * * Every 64 frames the serial port gets a line, `doom frame <cycles>
 *   fps <tenths>`, the mean over those frames.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "board.h"
#include "doomgeneric.h"
#include "doomkeys.h"
#include "scan.h"
#include "uart.h"

#define UART_AT 0x00003000u
#define SCAN_AT 0x00003280u
#define FRAME 0x42000000u
#define ROW 1024u
#define TOP 40u

static inline uint32_t rd(uint32_t at) { return *(volatile uint32_t *)at; }
static inline void wr(uint32_t at, uint32_t v) { *(volatile uint32_t *)at = v; }

uint64_t board_cycles(void)
{
	uint32_t hi, lo, again;
	do {
		__asm__ volatile("csrr %0, mcycleh" : "=r"(hi));
		__asm__ volatile("csrr %0, mcycle" : "=r"(lo));
		__asm__ volatile("csrr %0, mcycleh" : "=r"(again));
	} while (hi != again);
	return (uint64_t)hi << 32 | lo;
}

void uart_put(char c)
{
	while (rd(UART_AT + UART_TXDATA) & UART_TXDATA_FULL_MASK) {
	}
	wr(UART_AT + UART_TXDATA, (unsigned char)c);
}

int uart_get(void)
{
	uint32_t v = rd(UART_AT + UART_RXDATA);
	return v & UART_RXDATA_EMPTY_MASK ? -1 : (int)(v & 0xff);
}

/* Keys held down, each until its deadline in milliseconds. */
#define HELD 8
#define HOLD_MS 180
static struct {
	unsigned char key;
	uint32_t until;
	int down;
} held[HELD];

/* Events not yet given to the game. */
static struct {
	unsigned char key;
	int down;
} queue[32];
static unsigned qhead, qtail;

static void push(unsigned char key, int down)
{
	if (qtail - qhead < sizeof queue / sizeof queue[0]) {
		queue[qtail % 32].key = key;
		queue[qtail % 32].down = down;
		qtail++;
	}
}

static void press(unsigned char key, uint32_t now)
{
	for (int i = 0; i < HELD; i++) {
		if (held[i].down && held[i].key == key) {
			held[i].until = now + HOLD_MS;
			return;
		}
	}
	for (int i = 0; i < HELD; i++) {
		if (!held[i].down) {
			held[i].key = key;
			held[i].down = 1;
			held[i].until = now + HOLD_MS;
			push(key, 1);
			return;
		}
	}
}

/* A byte from the serial port as Doom's key, or nought. An escape
 * starts the arrows' sequences, `ESC [ A` to `ESC [ D`, when the rest
 * of it has come; alone it is Escape. */
static unsigned char key_of(int c)
{
	switch (c) {
	case 'w': return KEY_UPARROW;
	case 's': return KEY_DOWNARROW;
	case 'a': return KEY_LEFTARROW;
	case 'd': return KEY_RIGHTARROW;
	case 'f': return KEY_FIRE;
	case ' ': return KEY_USE;
	case '\r':
	case '\n': return KEY_ENTER;
	default:
		if ((c >= '0' && c <= '9') || c == 'y' || c == 'n')
			return c;
		return 0;
	}
}

static void poll_keys(void)
{
	uint32_t now = DG_GetTicksMs();
	int c;
	while ((c = uart_get()) >= 0) {
		unsigned char k;
		if (c == 0x1b) {
			int a = uart_get();
			if (a == '[') {
				int b = uart_get();
				k = b == 'A'   ? KEY_UPARROW
				    : b == 'B' ? KEY_DOWNARROW
				    : b == 'C' ? KEY_RIGHTARROW
				    : b == 'D' ? KEY_LEFTARROW
						: 0;
			} else {
				k = KEY_ESCAPE;
			}
		} else {
			k = key_of(c);
		}
		if (k)
			press(k, now);
	}
	for (int i = 0; i < HELD; i++) {
		if (held[i].down && (int32_t)(now - held[i].until) >= 0) {
			held[i].down = 0;
			push(held[i].key, 0);
		}
	}
}

void DG_Init(void)
{
	volatile uint32_t *fb = (volatile uint32_t *)FRAME;
	for (uint32_t y = 0; y < 480; y++)
		for (uint32_t x = 0; x < 640; x++)
			fb[y * ROW + x] = 0;
	wr(SCAN_AT + SCAN_BASE, FRAME);
	wr(SCAN_AT + SCAN_CTRL, SCAN_CTRL_SCAN_MASK);
}

static uint64_t last;
static uint32_t frames;

void DG_DrawFrame(void)
{
	volatile uint32_t *fb = (volatile uint32_t *)FRAME + TOP * ROW;
	const uint32_t *src = DG_ScreenBuffer;
	for (uint32_t y = 0; y < DOOMGENERIC_RESY; y++) {
		volatile uint32_t *a = fb + 2 * y * ROW, *b = a + ROW;
		const uint32_t *s = src + y * DOOMGENERIC_RESX;
		for (uint32_t x = 0; x < DOOMGENERIC_RESX; x++) {
			uint32_t p = s[x];
			a[2 * x] = p;
			a[2 * x + 1] = p;
			b[2 * x] = p;
			b[2 * x + 1] = p;
		}
	}
	if (++frames % 64 == 0) {
		uint64_t now = board_cycles();
		uint32_t each = (now - last) / 64;
		uint32_t tenths = (uint32_t)(10ull * BOARD_HZ / each);
		printf("doom frame %lu fps %lu.%lu\n", (unsigned long)each,
		       (unsigned long)tenths / 10, (unsigned long)tenths % 10);
		last = now;
	}
	poll_keys();
}

void DG_SleepMs(uint32_t ms)
{
	uint64_t until = board_cycles() + (uint64_t)ms * (BOARD_HZ / 1000u);
	while (board_cycles() < until)
		poll_keys();
}

uint32_t DG_GetTicksMs(void)
{
	return (uint32_t)(board_cycles() / (BOARD_HZ / 1000u));
}

int DG_GetKey(int *pressed, unsigned char *key)
{
	poll_keys();
	if (qhead == qtail)
		return 0;
	*key = queue[qhead % 32].key;
	*pressed = queue[qhead % 32].down;
	qhead++;
	return 1;
}

void DG_SetWindowTitle(const char *title) { (void)title; }

int main(void)
{
	printf("doom on vreteno\n");
	last = board_cycles();
	char *argv[] = {"doom", "-iwad", "doom.wad", NULL};
	doomgeneric_Create(3, argv);
	for (;;)
		doomgeneric_Tick();
}
