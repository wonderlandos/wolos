//! Tool output fixtures: an Alder Lake-P laptop (ThinkPad-class) on FreeBSD 15.1. Formats follow the
//! test VM's real output (`pciconf -lv`, `devinfo -v`, `kldstat -v`); USB per usbconfig(8).

pub const PCICONF_ADL: &str = "\
hostb0@pci0:0:0:0:\tclass=0x060000 rev=0x02 hdr=0x00 vendor=0x8086 device=0x4621 subvendor=0x17aa subdevice=0x22e6
    vendor     = 'Intel Corporation'
    device     = '12th Gen Core Processor Host Bridge/DRAM Registers'
    class      = bridge
    subclass   = HOST-PCI
vgapci0@pci0:0:2:0:\tclass=0x030000 rev=0x0c hdr=0x00 vendor=0x8086 device=0x46a8 subvendor=0x17aa subdevice=0x22e6
    vendor     = 'Intel Corporation'
    device     = 'Alder Lake-UP3 GT2 [Iris Xe Graphics]'
    class      = display
    subclass   = VGA
xhci0@pci0:0:13:0:\tclass=0x0c0330 rev=0x02 hdr=0x00 vendor=0x8086 device=0x461e subvendor=0x17aa subdevice=0x22e6
    vendor     = 'Intel Corporation'
    device     = 'Alder Lake-P Thunderbolt 4 USB Controller'
    class      = serial bus
    subclass   = USB
iwlwifi0@pci0:0:20:3:\tclass=0x028000 rev=0x01 hdr=0x00 vendor=0x8086 device=0x51f0 subvendor=0x8086 subdevice=0x0094
    vendor     = 'Intel Corporation'
    device     = 'Alder Lake-P PCH CNVi WiFi'
    class      = network
ig4iic0@pci0:0:21:0:\tclass=0x0c8000 rev=0x01 hdr=0x00 vendor=0x8086 device=0x51e8 subvendor=0x17aa subdevice=0x22e6
    vendor     = 'Intel Corporation'
    device     = 'Alder Lake PCH Serial IO I2C Controller #0'
    class      = serial bus
none1@pci0:0:31:3:\tclass=0x040380 rev=0x01 hdr=0x00 vendor=0x8086 device=0x51c8 subvendor=0x17aa subdevice=0x22e6
    vendor     = 'Intel Corporation'
    device     = 'Alder Lake PCH-P High Definition Audio Controller'
    class      = multimedia
    subclass   = HDA
re0@pci0:2:0:0:\tclass=0x020000 rev=0x15 hdr=0x00 vendor=0x10ec device=0x8168 subvendor=0x17aa subdevice=0x22e6
    vendor     = 'Realtek Semiconductor Co., Ltd.'
    device     = 'RTL8111/8168/8211/8411 PCI Express Gigabit Ethernet Controller'
    class      = network
    subclass   = ethernet
rtsx0@pci0:3:0:0:\tclass=0xff0000 rev=0x01 hdr=0x00 vendor=0x10ec device=0x525a subvendor=0x17aa subdevice=0x22e6
    vendor     = 'Realtek Semiconductor Co., Ltd.'
    device     = 'RTS525A PCI Express Card Reader'
nvme0@pci0:4:0:0:\tclass=0x010802 rev=0x00 hdr=0x00 vendor=0x144d device=0xa80a subvendor=0x144d subdevice=0xa801
    vendor     = 'Samsung Electronics Co Ltd'
    device     = 'NVMe SSD Controller PM9A1/PM9A3/980PRO'
    class      = mass storage
    subclass   = NVM
";

pub const DEVINFO_ADL: &str = "\
nexus0
  cryptosoft0 <software crypto>
  apic0 <APIC resources>
  acpi0 <LENOVO TP-R2B  >
    cpu0 pnpinfo _HID=none _UID=0 _CID=none at handle=\\_SB_.PR00
      hwpstate_intel0 <Intel Speed Shift>
      cpufreq0
    cpu1 pnpinfo _HID=none _UID=0 _CID=none at handle=\\_SB_.PR01
      hwpstate_intel1 <Intel Speed Shift>
      cpufreq1
    unknown pnpinfo _HID=ELAN0678 _UID=1 _CID=PNP0C50 at handle=\\_SB_.PC00.I2C0.TPD0
    pcib0 <ACPI Host-PCI bridge> pnpinfo _HID=PNP0A08 _UID=0 _CID=PNP0A03 at handle=\\_SB_.PC00
      pci0 <ACPI PCI bus>
        hostb0 <Host to PCI bridge> pnpinfo vendor=0x8086 device=0x4621 subvendor=0x17aa subdevice=0x22e6 class=0x060000 at slot=0 function=0 dbsf=pci0:0:0:0 handle=\\_SB_.PC00
        vgapci0 <VGA-compatible display> pnpinfo vendor=0x8086 device=0x46a8 subvendor=0x17aa subdevice=0x22e6 class=0x030000 at slot=2 function=0 dbsf=pci0:0:2:0 handle=\\_SB_.PC00.GFX0
          drmn0
        xhci0 <Intel Alder Lake-P Thunderbolt 4 USB controller> pnpinfo vendor=0x8086 device=0x461e subvendor=0x17aa subdevice=0x22e6 class=0x0c0330 at slot=13 function=0 dbsf=pci0:0:13:0 handle=\\_SB_.PC00.TXHC
          usbus0
            uhub0 <0x8086 XHCI root HUB, class 9/0, rev 3.00/1.00, addr 1> pnpinfo vendor=0x8086 product=0x0000 devclass=0x09 devsubclass=0x00 devproto=0x01 sernum=\"\" release=0x0100 mode=host intclass=0x09 intsubclass=0x00 intprotocol=0x00 at bus=0 hubaddr=0 port=0 devaddr=1 interface=0 ugen=ugen0.1
              ubt0 <Intel Bluetooth> pnpinfo vendor=0x8087 product=0x0026 devclass=0xe0 devsubclass=0x01 devproto=0x01 sernum=\"\" release=0x0002 mode=host intclass=0xe0 intsubclass=0x01 intprotocol=0x01 at bus=0 hubaddr=1 port=10 devaddr=3 interface=0 ugen=ugen0.3
        iwlwifi0 <iwlwifi> pnpinfo vendor=0x8086 device=0x51f0 subvendor=0x8086 subdevice=0x0094 class=0x028000 at slot=20 function=3 dbsf=pci0:0:20:3 handle=\\_SB_.PC00.CNVW
        ig4iic0 <Intel Alder Lake-M I2C Controller-0> pnpinfo vendor=0x8086 device=0x51e8 subvendor=0x17aa subdevice=0x22e6 class=0x0c8000 at slot=21 function=0 dbsf=pci0:0:21:0 handle=\\_SB_.PC00.I2C0
          iicbus0 <Philips I2C bus>
        unknown pnpinfo vendor=0x8086 device=0x51c8 subvendor=0x17aa subdevice=0x22e6 class=0x040380 at slot=31 function=3 dbsf=pci0:0:31:3 handle=\\_SB_.PC00.HDAS
        pcib1 <ACPI PCI-PCI bridge> at slot=28 function=0 dbsf=pci0:0:28:0 handle=\\_SB_.PC00.RP01
          pci1 <ACPI PCI bus>
            re0 <RealTek 8168/8111 B/C/CP/D/DP/E/F/G PCIe Gigabit Ethernet> pnpinfo vendor=0x10ec device=0x8168 subvendor=0x17aa subdevice=0x22e6 class=0x020000 at slot=0 function=0 dbsf=pci0:2:0:0 handle=\\_SB_.PC00.RP01.PXSX
              miibus0
            rtsx0 <2.0 Realtek RTS525A PCI MMC/SD Card Reader> pnpinfo vendor=0x10ec device=0x525a subvendor=0x17aa subdevice=0x22e6 class=0xff0000 at slot=0 function=0 dbsf=pci0:3:0:0 handle=\\_SB_.PC00.RP02.PXSX
";

pub const USBCONFIG_LAPTOP: &str = "\
ugen0.1: <Intel XHCI root HUB> at usbus0, cfg=0 md=HOST spd=SUPER (5.0Gbps) pwr=SAVE (0mA)

  bLength = 0x0012
  bDescriptorType = 0x0001
  bcdUSB = 0x0300
  bDeviceClass = 0x0009  <Hub>
  bDeviceSubClass = 0x0000
  bDeviceProtocol = 0x0003
  bMaxPacketSize0 = 0x0009
  idVendor = 0x8086
  idProduct = 0x0000
  bcdDevice = 0x0100
  iManufacturer = 0x0001  <Intel>
  iProduct = 0x0002  <XHCI root HUB>
  iSerialNumber = 0x0000  <no string>
  bNumConfigurations = 0x0001


 Configuration index 0

    bLength = 0x0009
    bDescriptorType = 0x0002
    bNumInterfaces = 0x0001

    Interface 0
      bLength = 0x0009
      bDescriptorType = 0x0004
      bInterfaceNumber = 0x0000
      bAlternateSetting = 0x0000
      bNumEndpoints = 0x0001
      bInterfaceClass = 0x0009  <Hub>
      bInterfaceSubClass = 0x0000
      bInterfaceProtocol = 0x0000
      iInterface = 0x0000  <no string>

ugen0.1.0: uhub0: <Intel XHCI root HUB, class 9/0, rev 3.00/1.00, addr 1>
ugen0.3: <vendor 0x8087 product 0x0026> at usbus0, cfg=0 md=HOST spd=FULL (12Mbps) pwr=ON (100mA)

  bLength = 0x0012
  bDeviceClass = 0x00e0  <Wireless Controller>
  bDeviceSubClass = 0x0001
  bDeviceProtocol = 0x0001
  idVendor = 0x8087
  idProduct = 0x0026
  bcdDevice = 0x0002

 Configuration index 0

    Interface 0
      bInterfaceNumber = 0x0000
      bInterfaceClass = 0x00e0  <Wireless Controller>
      bInterfaceSubClass = 0x0001
      bInterfaceProtocol = 0x0001

    Interface 1
      bInterfaceNumber = 0x0001
      bInterfaceClass = 0x00e0  <Wireless Controller>
      bInterfaceSubClass = 0x0001
      bInterfaceProtocol = 0x0001

ugen0.3.0: ubt0: <Intel Bluetooth>
ugen0.4: <Chicony Electronics Co.,Ltd. Integrated Camera> at usbus0, cfg=0 md=HOST spd=HIGH (480Mbps) pwr=ON (500mA)

  bDeviceClass = 0x00ef  <Miscellaneous device>
  bDeviceSubClass = 0x0002
  bDeviceProtocol = 0x0001
  idVendor = 0x04f2
  idProduct = 0xb6dd

 Configuration index 0

    Interface 0
      bInterfaceClass = 0x000e  <Video>
      bInterfaceSubClass = 0x0001
      bInterfaceProtocol = 0x0000

    Interface 1
      bInterfaceClass = 0x000e  <Video>
      bInterfaceSubClass = 0x0002
      bInterfaceProtocol = 0x0000

";

pub const KLDSTAT: &str = "\
Id Refs Address                Size Name
 1   51 0xffffffff80200000  22fbe60 kernel (/boot/kernel/kernel)
        Contains modules:
                 Id Name
                407 cubic
                262 pci/snd_hda
                263 hdac/snd_hda
                479 cpu/hwpstate_intel
                380 cpu/cpufreq
                232 pci/re
                133 pci/igc
                109 pci/em
                239 pci/sdhci_pci
 7    1 0xffffffff82c20000   621c30 zfs.ko (/boot/kernel/zfs.ko)
        Contains modules:
                 Id Name
                481 zfsctrl
 9    1 0xffffffff83618000    c4498 if_iwlwifi.ko (/boot/kernel/if_iwlwifi.ko)
        Contains modules:
                 Id Name
                510 iwlwifi
10    1 0xffffffff83800000   2c2160 i915kms.ko (/boot/modules/i915kms.ko)
11    1 0xffffffff83b00000     4250 rtsx.ko (/boot/kernel/rtsx.ko)
12    1 0xffffffff83b10000     9a30 ig4.ko (/boot/kernel/ig4.ko)
13    1 0xffffffff83b20000     b1c8 ng_ubt.ko (/boot/kernel/ng_ubt.ko)
";
