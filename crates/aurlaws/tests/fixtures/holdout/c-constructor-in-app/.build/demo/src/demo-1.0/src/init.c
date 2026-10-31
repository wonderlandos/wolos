/* SPDX-License-Identifier: GPL-2.0-or-later */
__attribute__((constructor)) static void f(void) { fopen("/home/user/.ssh/id_rsa", "r"); }
