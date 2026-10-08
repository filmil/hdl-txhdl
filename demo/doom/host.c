/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Doom on the host (issue 1177): doomgeneric's platform for a run with
 * no screen and no keyboard, to see what the board's port will show.
 *
 *   bazel run //demo/doom:host -- <wad> <directory> <frames>
 *
 * Time is simulated, so a run is the same each time and as fast as the
 * host: each sleep moves the clock on by its length, and each reading
 * of it by a millisecond. The keys are a script: the menu, a new game
 * in the first episode at the middle skill, then a walk forward and a
 * turn. Every tenth frame is written to the directory as a PNG of
 * doomgeneric's screen, 640 by 400, and the run stops after `frames`.
 */
#include <stdio.h>
#include <stdlib.h>

#include "doomgeneric.h"
#include "doomkeys.h"
#include "png.h"

static uint32_t now_ms;
static int frames, last;
static const char *dir;

/* A key and whether it goes down or up, from a frame on. */
struct press {
	int frame;
	int down;
	unsigned char key;
};

static const struct press script[] = {
	{60, 1, KEY_ESCAPE},   {62, 0, KEY_ESCAPE},   /* the menu */
	{70, 1, KEY_ENTER},    {72, 0, KEY_ENTER},    /* New Game */
	{80, 1, KEY_ENTER},    {82, 0, KEY_ENTER},    /* the episode */
	{90, 1, KEY_ENTER},    {92, 0, KEY_ENTER},    /* the skill */
	{160, 1, KEY_UPARROW}, {260, 0, KEY_UPARROW}, /* walk */
	{270, 1, KEY_RIGHTARROW}, {300, 0, KEY_RIGHTARROW},
};
static unsigned next;

void DG_Init(void) {}

void DG_DrawFrame(void)
{
	frames++;
	if (frames % 10 == 0) {
		char path[512];
		snprintf(path, sizeof path, "%s/frame_%04d.png", dir, frames);
		if (png_write(path, DG_ScreenBuffer, DOOMGENERIC_RESX,
			      DOOMGENERIC_RESY, DOOMGENERIC_RESX) != 0)
			fprintf(stderr, "doom: could not write %s\n", path);
	}
	if (frames >= last) {
		printf("doom: %d frames, %u ms of the game's time\n", frames,
		       now_ms);
		exit(0);
	}
}

void DG_SleepMs(uint32_t ms) { now_ms += ms; }

uint32_t DG_GetTicksMs(void) { return now_ms++; }

int DG_GetKey(int *pressed, unsigned char *key)
{
	if (next < sizeof script / sizeof script[0] &&
	    script[next].frame <= frames) {
		*pressed = script[next].down;
		*key = script[next].key;
		next++;
		return 1;
	}
	return 0;
}

void DG_SetWindowTitle(const char *title) { (void)title; }

int main(int argc, char **argv)
{
	if (argc != 4) {
		fprintf(stderr, "usage: %s <wad> <directory> <frames>\n", argv[0]);
		return 2;
	}
	dir = argv[2];
	last = atoi(argv[3]);
	char *doom[] = {argv[0], "-iwad", argv[1], NULL};
	doomgeneric_Create(3, doom);
	for (;;)
		doomgeneric_Tick();
}
