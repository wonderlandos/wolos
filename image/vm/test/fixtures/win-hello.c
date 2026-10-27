typedef void *HANDLE; typedef unsigned long DWORD;
__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
void mainCRTStartup(void) {
  static const char m[] = "Hello from a Windows console exe (WolOS wol-run -> Proton)\r\n";
  DWORD n; WriteFile(GetStdHandle((DWORD)-11), m, sizeof m - 1, &n, 0); ExitProcess(7);
}
