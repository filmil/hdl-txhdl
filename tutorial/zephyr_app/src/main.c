// SPDX-License-Identifier: Apache-2.0
//
// The smallest application from a package of its own: it says where it
// was built from, and opens a TCP socket, so that the image links the
// stack its prj.conf asked for.
#include <zephyr/kernel.h>
#include <zephyr/net/socket.h>

int main(void)
{
	printk("zephyr_app: built from a package of its own\n");
	int s = zsock_socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
	printk("zephyr_app: TCP socket %s\n", s >= 0 ? "open" : "refused");
	return 0;
}
