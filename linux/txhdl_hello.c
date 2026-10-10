// SPDX-License-Identifier: GPL-2.0
/*
 * The smallest loadable module (issue 1441): it says when it is loaded
 * and when it is removed, so a boot shows that the kernel loads
 * modules. GPL-2.0, unlike the rest of the tree, since a module is
 * built against the kernel and one under another licence taints it.
 */
#include <linux/init.h>
#include <linux/module.h>
#include <linux/printk.h>

static int __init txhdl_hello_init(void)
{
	pr_info("txhdl: a module loaded\n");
	return 0;
}

static void __exit txhdl_hello_exit(void)
{
	pr_info("txhdl: a module removed\n");
}

module_init(txhdl_hello_init);
module_exit(txhdl_hello_exit);
MODULE_DESCRIPTION("Says when it is loaded and removed (issue 1441)");
MODULE_LICENSE("GPL");
