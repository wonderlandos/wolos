// SPDX-License-Identifier: GPL-2.0-or-later
// The standard antivirus test file: harmless, and the way to see the whole path work.
rule Test_Standard_Test_File {
    strings:
        $a = "EICAR-STANDARD-ANTIVIRUS-TEST-FILE"
    condition:
        $a
}
