/* SPDX-License-Identifier: Apache-2.0 */
/* A program linked against musl's shared library (issue 1439): it says
 * so, and what the loader handed it, so the board shows that a dynamic
 * program loads and runs. */
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv)
{
	printf("txhdl: a dynamic program ran, %s, %zu bytes of argv[0]\n",
	       argc > 0 ? argv[0] : "?", argc > 0 ? strlen(argv[0]) : 0);
	return 0;
}
