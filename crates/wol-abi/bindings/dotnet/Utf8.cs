// SPDX-License-Identifier: GPL-2.0-or-later
using System.Runtime.InteropServices;

namespace Wol;

/// <summary>A string as UTF-8 bytes and a byte length, the way libwol takes text. Dispose frees it.</summary>
public readonly unsafe struct Utf8 : IDisposable
{
    public readonly byte* Ptr;
    public readonly nuint Len;

    public Utf8(string s)
    {
        Ptr = (byte*)Marshal.StringToCoTaskMemUTF8(s);
        Len = (nuint)System.Text.Encoding.UTF8.GetByteCount(s);
    }

    public void Dispose() => Marshal.FreeCoTaskMem((IntPtr)Ptr);
}
