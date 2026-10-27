/* CRT-free Windows Vulkan probe (image/vm/test/games.sh): winevulkan -> the host's Vulkan ICD,
   the path DXVK and vkd3d-proton take. No window. Prints the devices; exit 0 = at least one,
   1 = vkCreateInstance refused, 2 = no device. Build: clang --target=x86_64-pc-windows-msvc. */
typedef void *HANDLE; typedef unsigned long DWORD;
__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);

typedef struct {
  int sType; const void *pNext; unsigned flags; const void *app;
  unsigned layers; const char *const *layer_names; unsigned exts; const char *const *ext_names;
} VkInstanceCreateInfo;
__declspec(dllimport) int __stdcall vkCreateInstance(const VkInstanceCreateInfo *, const void *, void **);
__declspec(dllimport) int __stdcall vkEnumeratePhysicalDevices(void *, unsigned *, void **);
__declspec(dllimport) void __stdcall vkGetPhysicalDeviceProperties(void *, void *);

static void put(const char *s) {
  DWORD n = 0, len = 0;
  while (s[len]) len++;
  WriteFile(GetStdHandle((DWORD)-11), s, len, &n, 0);
}

static void put_u(unsigned v) {
  char b[12]; int i = 11; b[i] = 0;
  do { b[--i] = (char)('0' + v % 10); v /= 10; } while (v);
  put(b + i);
}

/* VkPhysicalDeviceProperties is 824 bytes on x86_64; deviceName starts at byte 20. */
static unsigned long long props[128];

void mainCRTStartup(void) {
  static const char *const types[] = {"other", "integrated", "discrete", "virtual", "cpu"};
  VkInstanceCreateInfo ci = {1, 0, 0, 0, 0, 0, 0, 0}; /* VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO */
  void *inst = 0, *dev[8];
  unsigned count = 8, i;
  int r = vkCreateInstance(&ci, 0, &inst);
  if (r != 0) { put("vulkan: vkCreateInstance refused\r\n"); ExitProcess(1); }
  vkEnumeratePhysicalDevices(inst, &count, dev);
  put("vulkan: "); put_u(count); put(" device(s)\r\n");
  for (i = 0; i < count; i++) {
    const unsigned *p = (const unsigned *)props;
    vkGetPhysicalDeviceProperties(dev[i], props);
    put("vulkan: "); put((const char *)props + 20);
    put(" ("); put(p[4] < 5 ? types[p[4]] : "?"); put(", Vulkan ");
    put_u(p[0] >> 22); put("."); put_u((p[0] >> 12) & 0x3ff); put(")\r\n");
  }
  ExitProcess(count ? 0 : 2);
}
