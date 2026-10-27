# The Wonderland License, Version 1.0

SPDX identifier: `LicenseRef-Wonderland-1.0`
Licensor: Julia Klee and her company (with affiliates and successors)
Copyright (c) 2026 Julia Klee. Everyone may copy and distribute this license text verbatim.

---

## Preamble

WolOS is built in the open. The Wonderland License gives everyone the freedom to use, study, change, run, share and sell the WolOS kernel and driver innovations: performance work on the FreeBSD kernel and the graphics drivers, written so that every program on a WolOS machine feels fast. The source is published for anyone to read and build. We want people to learn from it, measure it, improve it and build products with it, and we want every improvement to stay available to everyone in the same way.

This License has two parts. **Part I (Sections 0 to 3)** sets out Your freedoms and our promises. **Part II (Sections 4 to 12) sets out the conditions** that come with those freedoms. Please read both: the conditions are binding.

## Summary (Non-Binding)

> **This summary is not part of the License and has no legal effect.** If the summary and the License disagree, the License wins.

This License covers the WolOS kernel and driver innovations: the kernel and graphics driver patches, and the low-level kernel modules that go with them. Other parts of WolOS have their own licenses, and the WolOS brand art has the Wonderland Brand License. The Covered Work is source-available.

**Your freedoms (Section 2)**

- Use it for anything: at home, at work, in a school, in a data center, in a product.
- Read and study every line, build it yourself, and learn from it.
- Change it, in any way, for any reason.
- Run it anywhere: on your own machines, your company's, your customers', in the cloud.
- Use it commercially: sell it, support it, consult on it, put it in devices you sell.
- Share it, changed or unchanged, with anyone.
- Combine it with your own code under any license you like; only the files you got from us stay under this License.
- Benchmark it and publish the results, good or bad, without asking anyone.
- Fork it.
- Contribute back, and keep the copyright in what you write.

**Our promises (Section 3)**

- A patent license from every contributor, for the work they contributed.
- No surprise termination: a first mistake can be fixed within 30 days with your rights restored, and the people you shared with keep theirs.
- The source stays public, so the whole community keeps the same freedoms you have.

**Conditions (Part II, Sections 4 to 12)**

- **Notices (Section 10):** keep the copyright notices, this License and the `NOTICE` file with the work.
- **Source of changes, copyleft (Section 4):** when you share or deploy changed Covered files, publish their source under this License, with a short note of what changed, and say where to find it. Changes stay under this License.
- **Other operating systems (Section 4.2A):** an operating system other than an official WolOS build that ships these kernel innovations publishes the source of those kernel parts within 30 days and credits "Contains WolOS kernel innovations by Julia Klee".
- **No re-derivation (Section 4.2B):** the kernel innovations' source may not be used as a guide to write a look-alike under other terms.
- **License-backs (Sections 6.1, 6.8, 6.9):** changes you ship or send in come with a license, including patents, to the Licensor.
- **Names and art (Sections 7, 8, 9):** the names WolOS, Wonderland OS and WolKernel, the logo and the characters are not licensed here; forks use their own name, and results of modified builds are labelled as such.
- **Patent suits (Section 3.2):** suing over patents in the Covered Work ends the suing party's patent license under it.
- **Termination (Section 12):** breaking these conditions ends the License, with a 30-day cure for a first breach.

The binding terms follow.

---

## Table of Contents

**Part I: Freedoms and Promises**
0. Scope
1. Definitions
2. Your Freedoms
3. Our Promises

**Part II: Conditions**
4. Conditions on Distribution and Deployment (source, copyleft, other operating systems, re-derivation)
5. Larger Works
6. Contributions and License-Backs
7. Brand Art Not Covered
8. Trademarks
9. Public Claims About Results
10. Notices and Attribution
11. Compatibility with Other Licenses
12. Termination

**Part III: Disclaimers and General Terms**
13. No Warranty
14. Limitation of Liability
15. Indemnity for Commercial Distributors
16. Export and Sanctions Compliance
17. Governing Law and Jurisdiction
18. Versions of This License
19. General Terms
Appendix A: File Header Notices
Appendix B: NOTICE File Template

---

# Part I: Freedoms and Promises

## 0. Scope

0.1 **Covered.** This License applies only to:
  (a) the WolOS kernel performance patches (`kernel/patches/`, with their test programs in `kernel/patches/tests/`);
  (b) the WolOS patches to the DRM / i915 graphics driver (`kernel/drm/patches/`);
  (c) the WolOS kernel modules and low-level kernel work in `kernel/wolos_qos/`, and the scripts that check the patches and the kernel and driver built from them (`kernel/verify.sh`, `kernel/drm/verify.sh`, `kernel/tests/`); and
  (d) any other low-level kernel or driver work the Licensor marks with this License under Appendix A.

  Nothing else is covered. In particular, art, logos, icons, wallpapers, illustrations, characters, sounds, fonts and design tokens are never covered by this License, even where they appear in or next to a Covered File.

0.2 **Not covered.** All other WolOS components carry their own licenses and are not affected by this License. In particular: the WolOS shell (menu bar, Dock, Control Center, Settings, Kit) stays under GPL-3.0-or-later; WolOS's own userland code (system API, daemons, CLI, image and build scripts) stays under BSD-2-Clause; the WolOS brand art (the `design/` tree and `assets/`) is under the Wonderland Brand License, Version 1.0 (`LicenseRef-Wonderland-Brand-1.0`); third-party software stays under its own terms.

0.3 **The file notice decides.** A file is Covered only if it carries a notice under Appendix A, or sits in a directory listed in 0.1 whose license file names this License. If a file's own notice names a different license, that notice controls.

## 1. Definitions

In this License, the following capitalized terms have these meanings. Singular includes plural and the reverse.

1.1 **"License"** means this document, The Wonderland License, Version 1.0, or a later version chosen under Section 18.

1.2 **"Licensor"** means Julia Klee and any company she controls that holds or administers the rights in the Original Work, together with their Affiliates, successors and assignees. All rights reserved, licensed back or granted to "the Licensor" under this License benefit each of them.

1.2A **"Affiliate"** means an entity that controls, is controlled by, or is under common control with the Licensor (control as defined in Section 1.3).

1.3 **"You"** (or **"Your"**) means the individual or legal entity exercising rights under this License. For entities, "You" includes every entity that controls, is controlled by, or is under common control with You. "Control" means direct or indirect ownership of more than fifty percent (50%) of the voting interests, or the power to direct management by contract or otherwise.

1.4 **"Original Work"** means the works the Licensor makes available under this License, as identified by a notice in or attached to the work (see Appendix A). The Original Work consists of Code only.

1.5 **"Code"** means the parts of a Covered Work that are software: source code, patch files, diffs, build and configuration scripts, and any object or executable form produced from them.

1.6 **"Brand Art"** means visual or design works of the Licensor: logos, icons, glyphs, wallpapers, illustrations, character designs, animations, sounds, fonts, color palettes and design tokens, and their source files, in any format or resolution. Brand Art is not Covered Work.

1.7 **"Covered Work"** means the Original Work, Modifications, and combinations of the two, in each case only to the extent they are governed by this License. A Covered Work does not include parts of a Larger Work that are not Covered Files.

1.8 **"Covered File"** means any individual file (or, for patches, any individual hunk applied to a target file) that contains, in whole or in part, the Original Work or a Modification.

1.9 **"Modification"** means any of the following:
  (a) any addition to, deletion from, or change to the content of a Covered File;
  (b) any new file that contains a substantial portion of a Covered File, or that is derived from it;
  (c) for patches, any change to the lines a patch adds or alters, including a port of the same change to another version of the target source;
  (d) any Kernel Derivative.

  A Modification does not include a new, separate file that only calls, links to, loads, or interoperates with a Covered File without copying its protected expression.

1.9A **"Kernel Innovation"** means any Covered Code under Section 0.1(a), and the specific technical methods, algorithms and code structures it expresses.

1.9B **"Kernel Derivative"** means any code that implements a Kernel Innovation and was written by copying, translating, porting or adapting Covered Code, or by using Covered Code as a reference or specification during its development, in any kernel, driver, or programming language.

1.9C **"Competing OS"** means any operating system, distribution, kernel, or firmware platform for general-purpose computers, other than an Official Build, that is Distributed or Deployed to third parties.

1.10 **"Larger Work"** means a work that combines a Covered Work with material not governed by this License, such as the FreeBSD source tree, a driver tree, an application, or a firmware image.

1.11 **"Target Source"** means third-party source code to which a Covered patch is applied, such as the FreeBSD kernel or the drm-kmod driver. Target Source is not a Covered Work and stays under its own license.

1.12 **"Contributor"** means the Licensor and each person or entity that submits a Contribution that the Licensor accepts.

1.13 **"Contribution"** means any work of authorship, including Modifications, intentionally submitted to the Licensor for inclusion in a Covered Work, by any means (for example a pull request, patch, issue attachment or e-mail). A communication clearly marked by its author as "Not a Contribution" is not a Contribution.

1.14 **"Distribute"** means to make a copy of a Covered Work, in any form, available to any third party, by any means, including sale, download, physical media, and pre-installation on a device.

1.15 **"Deploy"** means to:
  (a) Distribute a Covered Work;
  (b) use a Covered Work, or a Larger Work containing it, to provide a service, feature or function to any third party over a network (including hosted, cloud, streaming or remote desktop services), whether or not for a fee; or
  (c) ship, lease, rent, or otherwise provide a device or system on which a Covered Work is installed or runs, to any third party.

  Use solely by You, Your employees and Your contractors working on Your behalf, for Your internal purposes, is not Deployment.

1.16 **"Source Form"** means the preferred form for making changes: human-readable source and patch files with the scripts used to apply and build them.

1.17 **"Executable Form"** means any form other than Source Form, including binaries, kernel modules and kernel images.

1.18 **"Licensed Patents"** means patent claims, now owned or later acquired, licensable by a Contributor, that would be infringed by making, using, selling, offering for sale, importing or otherwise transferring that Contributor's Contribution, alone or as combined by that Contributor with the Covered Work to which it was submitted. Licensed Patents do not include claims infringed only by later Modifications made by others, or only by combination with material that was not part of that Covered Work.

1.19 **"Marks"** means the names "WolOS", "Wonderland OS", "WolKernel", "Wonderland" when used for software or devices, the WolOS logo and logotype, the characters (including the White Rabbit, the Keyhole, the Cheshire, the Pocket Watch, the Teacup, and any other character, mascot or persona the Licensor publishes for WolOS), and any confusingly similar name, logo, character or trade dress, whether registered or not.

1.20 **"Official Build"** means a build of WolOS published by the Licensor or by a party the Licensor authorizes in writing.

1.21 **"Unofficial"** means not an Official Build.

---

## 2. Your Freedoms

2.1 **Copyright grant.** Subject to the terms of this License, the Licensor and each Contributor grant You a worldwide, royalty-free, non-exclusive, non-sublicensable (except as Section 4.6 allows) license, for the duration of the copyright in the Covered Work, to:
  (a) use, reproduce, display and perform the Covered Work;
  (b) create Modifications and Larger Works;
  (c) Distribute and Deploy the Covered Work, in Source Form or Executable Form, alone or as part of a Larger Work;

  for any purpose, including commercial purposes.

2.1A **Your freedoms, in detail.** Without limiting Section 2.1, the grant includes each of the following freedoms, subject only to the conditions in Part II:
  (a) **Use.** To use the Covered Work for any purpose, by any person or organization, in any field of endeavor, including personal, educational, research, governmental, non-profit and commercial use. For example: a student running WolKernel on a laptop, a studio using it on render machines, or a company using it on every office desktop.
  (b) **Study.** To read, inspect, audit and study the Source Form; to build it Yourself; to trace, debug and profile it; and to teach and write about how it works, for example in courses, books, articles, conference talks and code reviews.
  (c) **Modify.** To change the Covered Work in any way and for any reason, for example to fix bugs, port it to another version of the Target Source, tune it for other hardware, or add features.
  (d) **Run anywhere.** To run the Covered Work, modified or not, on any number of machines that You or others own or operate, including workstations, laptops, servers, virtual machines, embedded devices and cloud instances, in any country.
  (e) **Commercial use.** To sell, license, rent, support, maintain, consult on, offer training for, and otherwise earn money with the Covered Work and with products and services that include it (see also Sections 3.8 and 4.2C).
  (f) **Redistribute.** To copy and share the Covered Work, modified or not, in Source Form or Executable Form, gratis or for a fee, by any medium, including downloads, package repositories, installation images, mirrors and physical media.
  (g) **Ship in products.** To include the Covered Work in Larger Works, distributions, appliances and devices You make, sell or lease, and to combine it with Your own code, under terms of Your choice for the files that are not Covered Files (Section 5).
  (h) **Benchmark.** To run, publish and discuss benchmarks, tests, reviews and comparisons of the Covered Work, including unfavorable results, without asking anyone. No clause of this License restricts truthful public commentary.
  (i) **Fork.** To start and maintain Your own version of the Covered Work, under Your own name, and to develop it in any direction.
  (j) **Contribute.** To send improvements to the Licensor, keeping the copyright in Your Contribution (Section 3.10).
  (k) **Private changes.** To make and use Modifications privately, inside Your organization, without publishing them (Section 1.15, last paragraph).

2.1B **The freedoms, stated at length.** For the avoidance of any doubt, and so that no freedom granted by this License is ever read narrowly, the Licensor and each Contributor declare and confirm that the license in Sections 2.1 and 2.1A is intended to be, and shall be construed as, the broadest grant of rights that the Licensor is able to give in the Covered Work consistent with Part II, and in particular that it includes, without limitation and in addition to everything already stated:
  (a) **the freedom to use**, which means the freedom of every natural person and every legal entity, of any size, nationality, sector or purpose, whether acting for itself or for others, for profit or not for profit, publicly or privately, to install, load, execute, operate, run, rely on and otherwise use the Covered Work, in whole or in part, alone or together with any other software, on any hardware, for any task, at any time, in any place, for any length of time and as many times and on as many machines as that person or entity wishes, without notice to, registration with, payment to, or permission from the Licensor or anyone else;
  (b) **the freedom to study**, which means the freedom to obtain, read, examine, inspect, analyze, audit, review, annotate, measure, test, trace, profile, debug, disassemble, decompile and otherwise study the Covered Work in Source Form and in Executable Form, in order to understand how it works, why it works, how well it works and how it could work better; to compile and build it from its Source Form with any tools; and to teach, explain, describe, discuss, quote and write about it, at any length and in any medium, including lectures, courses, curricula, textbooks, papers, theses, blog posts, videos and conference presentations;
  (c) **the freedom to modify**, which means the freedom to change, adapt, extend, reduce, refactor, translate, port, optimize, fix, rewrite and otherwise modify the Covered Work, in whole or in part, in any manner, to any extent and for any reason, including to make it run on other versions of the Target Source, on other processor architectures, on other graphics devices or on other machines, to correct defects, to add or remove features, to tune its behavior, and to combine it with other works;
  (d) **the freedom to run anywhere**, which means the freedom to deploy and run the Covered Work, modified or unmodified, on any number of computers, devices and systems, whether physical or virtual, owned, leased, rented or operated by You or by any other person, including desktops, laptops, workstations, servers, clusters, embedded and industrial devices, virtual machines, containers and cloud or hosted instances, in any country or territory;
  (e) **the freedom of commercial use**, which means the freedom to use, ship, sell, resell, license, lease, rent, bundle, pre-install, support, maintain, host, operate, consult on, train others in, certify, and otherwise derive revenue or commercial advantage from the Covered Work and from any product, device or service that includes or relies on it, for any price or for no price, without any royalty, fee or revenue share owed to the Licensor;
  (f) **the freedom to redistribute**, which means the freedom to make and give away or sell any number of copies of the Covered Work, modified or unmodified, in Source Form or Executable Form, to anyone, by any means of distribution now known or later developed, including downloads, mirrors, package repositories, version control hosting, installation images, live media, physical media, pre-installation on devices, and over-the-air updates;
  (g) **the freedom to ship in products**, which means the freedom to incorporate, integrate, embed and include the Covered Work in any Larger Work, operating system, distribution, appliance, device, firmware image or product line, and to license the parts of that Larger Work that are not Covered Files under any terms whatsoever;
  (h) **the freedom to benchmark and to speak**, which means the freedom to measure, test, compare and evaluate the Covered Work against any other software or system, and to publish, present, discuss and comment on the methods and results, favorable or unfavorable, without seeking review, approval or permission from anyone;
  (i) **the freedom to fork**, which means the freedom to create, name (subject only to Section 8), maintain, develop, govern and distribute an independent version of the Covered Work, and to take it in any technical direction; and
  (j) **the freedom to contribute and to share in the work**, which means the freedom to propose, submit and discuss improvements in the open, to keep the copyright in what You write, and to see Your improvements, and those of everyone else, remain available to all.

2.1C **Freedoms read broadly.** Every freedom in this Section is independent of, and cumulative with, every other freedom in it. No example given in this License limits the general words it illustrates. Where a use of the Covered Work is not expressly addressed by this License, and is not restricted by a condition in Part II, that use is permitted. The conditions in Part II limit these freedoms only to the extent their words clearly require.

2.2 **No art.** This License grants no right in Brand Art (see Section 7).

2.3 **Marks are excluded.** No right in the Marks is granted except as stated in Section 8.

2.4 **Effective date.** The grants take effect for each Contribution when the Contributor first makes it available.

2.5 **Reserved rights.** All rights not expressly granted are reserved by the Licensor and the Contributors. No license arises by implication, estoppel or otherwise.

2.6 **Fair use and statutory rights.** Nothing in this License limits rights You have under applicable law, such as fair use, fair dealing, or the limitations in Articles 46 to 48 of Brazilian Law No. 9,610/1998.

2.7 **Target Source.** This License grants no rights in Target Source and imposes no terms on it. You receive Target Source under its own license (for example, the BSD licenses of FreeBSD). Applying a Covered patch to Target Source does not change the license of the Target Source.

---

## 3. Our Promises

3.1 **Patent grant to You.** Subject to the terms of this License, each Contributor grants You a worldwide, royalty-free, non-exclusive license under its Licensed Patents to make, have made, use, sell, offer for sale, import and otherwise transfer its Contributions and the Covered Work, alone or in a Larger Work.

3.2 **Defensive termination.** If You (or any entity acting on Your behalf or at Your direction) initiate or join a claim, counterclaim or cross-claim in any court or agency (including a patent office proceeding other than a defensive response) alleging that a Covered Work, or any Contribution in it, directly or indirectly infringes a patent, then:
  (a) all patent licenses granted to You under this License terminate on the date the claim is filed; and
  (b) all patent licenses granted by You, as a Contributor, remain in force for everyone else.

3.3 **Narrow scope of the trigger.** Section 3.2 does not apply to a claim brought against a party that first sued You for patent infringement concerning the Covered Work, or to a declaratory judgment action asserting that Your own product does not infringe.

3.4 **Copyright survives patent termination.** Termination under Section 3.2 does not end Your copyright license under Section 2 unless Section 12 also applies.

3.5 **No other patents.** No license is granted under any patent for code not part of the Covered Work, or for Modifications made by You, or for the combination of a Covered Work with other material, except as Section 3.1 states.

3.6 **No surprise termination.** Your rights end only as Section 12 states: after a first breach You keep them if You cure it within thirty (30) days (Section 12.2); people who received the Covered Work from You keep their rights while they comply (Section 12.5); and end-user licenses You granted before termination survive (Section 12.6).

3.7 **Same freedoms for everyone.** Every recipient receives the same license directly from the Licensor and the Contributors (Section 4.6), and the Source Form of every Covered File that is Distributed or Deployed stays available to the public (Section 4.2), so improvements made by anyone remain available to everyone.

3.8 **Commercial use is welcome.** The Licensor will not treat commercial use of the Covered Work that complies with this License as a breach of it.

3.9 **Stable terms.** Only the Licensor may publish new versions of this License, and You may always keep using a Covered Work under the version You received it under (Section 18).

3.10 **Community.** You keep the copyright in Your Contributions and may use them in any way You like, apart from the Licensor's work (Section 6.7). The Licensor will credit accepted Contributions in the project history.

---

---

# Part II: Conditions

**The freedoms in Part I are granted subject to the conditions in this Part II (Sections 4 to 12). They are binding.**

**The conditions, in plain words.** Each line below is a binding condition, stated in full in the Section named:
1. If You share or deploy the Covered Work, keep its notices, this License and the `NOTICE` file (Section 10).
2. If You share or deploy a changed Covered File, publish its source under this License, mark what You changed, and say where the source is (Sections 4.1 to 4.4).
3. Changed Covered Files, and re-implementations written from the Covered source, stay under this License only (Sections 4.1A and 4.2B).
4. An operating system other than an official WolOS build that ships the kernel innovations publishes the source of those kernel parts within 30 days and shows "Contains WolOS kernel innovations by Julia Klee" (Section 4.2A).
5. Changes You ship or submit come with a license, including patents, to the Licensor (Sections 6.1, 6.8 and 6.9).
6. This License gives no right in the WolOS names, logo, characters or art; forks use their own name (Sections 7 and 8).
7. Benchmark results of a modified build are labelled as modified (Section 9).
8. A patent suit over the Covered Work ends the suing party's patent license under it (Section 3.2).
9. Breaking a condition ends this License, with 30 days to cure a first breach (Section 12).

## 4. Conditions on Distribution and Deployment

4.1 **The file-level rule.** Each Covered File, and each Modification of it, that You Distribute or Deploy stays under this License. You may not offer or impose terms that restrict or alter the recipient's rights in it under this License.

4.1A **Strong copyleft for Covered Work.** Every Modification You Distribute or Deploy, including every Kernel Derivative, must be released under this License in Source Form. You may not Distribute or Deploy any Modification under any other license, and may not sublicense it, except as Section 6.1 permits the Licensor.

4.2 **Source availability.** If You Deploy a Covered Work in any form, You must make the Source Form of every Covered File You Deploy, including Your Modifications, available under this License:
  (a) at no charge beyond a reasonable cost of physical media, through a publicly accessible location on the internet (for example a public source repository), or a written offer valid for at least three (3) years; and
  (b) no later than the date You first Deploy it, and for at least twelve (12) months after the last date You Deploy that version, or for as long as You Deploy it, whichever is longer.

4.2A **Kernel Innovations in a Competing OS.** If You ship a Kernel Innovation or a Kernel Derivative as part of a Competing OS, You must also:
  (a) publish the complete corresponding Source Form of every kernel and driver file that contains or links to it (including build configuration needed to rebuild those parts), under this License for the Covered parts, within thirty (30) days after each release, and keep it public as Section 4.2(b) requires;
  (b) include, in the release notes, the about or legal screen, and the source repository README, the attribution: "Contains WolOS kernel innovations by Julia Klee", with a link to the Original Work;
  (c) grant the Licensor the licenses in Sections 6.8 and 6.9 for those parts;
  (d) not relicense, dual-license, or sublicense those parts under other terms, nor ship them only in Executable Form.

4.2B **No reference-based re-derivation.** You agree not to use the Source Form of a Kernel Innovation as a reference, specification, or guide to create a functionally equivalent implementation released under terms other than this License. Code made that way is a Kernel Derivative. This covenant does not restrict: independent work by people who did not study the Covered source for that purpose; reading the source to learn, review, debug, interoperate or benchmark; or implementing ideas published elsewhere (for example in papers, mailing lists or other projects).

4.2C **Commercial use is welcome.** Nothing in Sections 4.1A to 4.2B restricts commercial use. Companies may use, ship, sell and support products that include the Covered Work, provided they comply with this License.

4.3 **Change notes.** Each Modified Covered File must carry a prominent notice stating that You changed it, the date of the change, and Your name or the name of Your entity. You must also include, with the Source Form, a plain-text description of the nature of Your changes (a change log, commit history or `CHANGES` file is enough).

4.4 **Telling people.** Wherever You Deploy, You must include, in documentation, an "About" screen, legal notices screen, or similar place that users normally see:
  (a) a statement that the product contains Covered Work under The Wonderland License 1.0;
  (b) where the Source Form can be obtained; and
  (c) the contents of the `NOTICE` file, if one exists.

4.5 **Executable Form.** You may Distribute the Covered Work in Executable Form under terms of Your choice, provided that (a) You meet Sections 4.2 to 4.4, and (b) those terms do not limit or alter the recipient's rights in the Source Form under this License, and state that any differing terms are offered by You alone.

4.6 **Passing it on.** Each time You Distribute a Covered Work, the recipient automatically receives a license from the Licensor and the Contributors under this License. You need not and may not grant a separate license to the Covered Files. You are not responsible for enforcing compliance by third parties.

4.7 **No added restrictions.** You may not add technical measures (such as signature checks, locked bootloaders, or encrypted kernel images) whose purpose or effect is to prevent recipients from replacing a Covered Work in a device You Deploy with a Modified version, unless You provide the recipient with the information needed to do so. This Section does not require You to support, warrant, or keep providing updates for such a replaced version, and it does not apply to devices sold only to businesses for their internal use.

4.8 **Additional terms of Your own.** You may offer, for a fee or not, warranty, support, indemnity or liability obligations to recipients, but only on Your own behalf and not on behalf of the Licensor or any Contributor. You must make clear that any such offer is Yours alone, and You agree to hold the Licensor and Contributors harmless from any liability arising from it.

---

## 5. Larger Works

5.1 **Combining.** You may create a Larger Work and Distribute or Deploy it under terms of Your choice, provided that the Covered Files in it continue to meet the requirements of this License.

5.2 **No reach beyond the files.** This License does not apply to, and imposes no obligation on, files in a Larger Work that are not Covered Files, even if they are compiled, linked, packaged or shipped together with Covered Files.

5.3 **Patches in a target tree.** When a Covered patch is applied to Target Source, only the lines added or altered by the patch (and their Modifications) are Covered Work. The rest of each patched file stays under the Target Source license. When You Distribute the patched file, You must keep the Covered lines identifiable by keeping the patch itself available in Source Form under Section 4.2.

5.4 **WolOS itself.** Parts of WolOS released by the Licensor under other licenses (for example BSD-2-Clause or GPL-3.0-or-later) remain under those licenses. Shipping Covered Work alongside them does not change their terms, and their terms do not change this License.

---

## 6. Contributions and License-Backs

6.1 **License-back.** By submitting a Contribution, You grant the Licensor, and anyone receiving software from the Licensor, a perpetual, worldwide, irrevocable, royalty-free, non-exclusive, transferable and sublicensable license to use, reproduce, modify, display, perform, Distribute, Deploy and sublicense Your Contribution, and works based on it, **under any terms the Licensor chooses**, including this License, other open-source or source-available licenses, and proprietary licenses.

6.2 **Patent license for Contributions.** You also grant the Licensor and all recipients the patent license in Section 3.1 for Your Contribution, and grant the Licensor the patent license in Section 6.8.

6.3 **Moral rights.** To the extent permitted by applicable law, You waive, or agree not to assert, moral rights in Your Contribution that would prevent the uses in Section 6.1. Where waiver is not permitted (including under Article 27 of Brazilian Law No. 9,610/1998), You agree that the Licensor may modify Your Contribution and that credit in the project history satisfies any attribution right.

6.4 **Your representations.** By submitting a Contribution, You represent that:
  (a) it is Your original work, or You have the right to submit it under this Section;
  (b) if Your employer or any other party has rights in it, You have their permission, or they have waived those rights;
  (c) it does not, to Your knowledge, infringe any third party's copyright, patent, trademark or trade secret;
  (d) it does not include code copied from works under licenses that are incompatible with this License (including, for patches to BSD-licensed Target Source, any code under the GNU GPL or LGPL);
  (e) You will tell the Licensor promptly if any of these become untrue.

6.5 **Sign-off.** The Licensor may require each Contribution to carry a `Signed-off-by:` line or a signed contributor agreement confirming Section 6.4.

6.6 **No obligation.** The Licensor need not accept, use, credit or keep any Contribution. Contributions are provided "AS IS" under Section 13, except for the representations in Section 6.4.

6.7 **Your own rights.** You keep the copyright in Your Contribution, and You may use it in any way You like, apart from the Licensor's work.

6.8 **Patent license-back to the Licensor.** If You Distribute or Deploy a Modification, or submit a Contribution, You grant the Licensor a perpetual, worldwide, irrevocable, royalty-free, non-exclusive, transferable and sublicensable license under all patent claims You own or control, now or later, that would be infringed by that Modification or Contribution, or by its combination with the Covered Work, to make, have made, use, sell, offer for sale, import and otherwise exploit it under any terms the Licensor chooses. Section 3.2 does not end this license.

6.9 **Copyright license-back for Modifications.** If You Distribute or Deploy a Modification, You grant the Licensor the license in Section 6.1 for that Modification, as if it were a Contribution, with the representations in Section 6.4.

---

## 7. Brand Art Not Covered

7.1 **Separate license.** Brand Art, including Brand Art shipped inside WolOS, inside a Larger Work, or next to Covered Files, is not licensed under this License. It is licensed, if at all, only under the Wonderland Brand License, Version 1.0, or a written permission of the Licensor.

7.2 **No implied art license.** Receiving, using or Distributing a Covered Work under this License gives no right in Brand Art or in the Marks beyond what Section 8 states.

---

## 8. Trademarks

8.1 **No trademark license.** This License does not grant any right to use the Marks, except as stated in this Section.

8.2 **Truthful reference.** You may use the Marks in text to truthfully refer to WolOS, state that Your work is based on, derived from, or compatible with WolOS, or describe the Covered Work's origin (for example "based on WolOS" or "runs on WolOS"). The reference must not be more prominent than Your own name.

8.3 **No endorsement.** You may not use the Marks in any way that suggests the Licensor sponsors, endorses, certifies or is affiliated with You, Your product or Your Modifications, unless she has agreed in writing.

8.4 **Forks must rename.** A Modified version that You Deploy may not be called "WolOS", "Wonderland OS" or "WolKernel", or any confusingly similar name, unless it is an Official Build. You may describe it as "a fork of WolOS".

8.5 **Packaging unchanged builds.** You may Distribute unmodified Official Builds under their original name.

8.6 **Domains and handles.** You may not register domain names, social media accounts, app store names or company names that contain the Marks, or are confusingly similar, without written permission.

8.7 **Guidelines.** The Licensor may publish trademark guidelines that give more permissions. Those guidelines may expand, but not narrow, the rights in this Section.

---

## 9. Public Claims About Results

9.1 **Publishing is free.** Section 2.1A(h) applies: benchmarks and reviews need no permission. This Section only keeps results attributed to the build that produced them.

9.2 **Say what you ran.** When You publish benchmark or performance results:
  (a) results from a Modified build, or from a build with settings changed from an Official Build's defaults, may not be presented as results of "WolOS" or "WolKernel" without stating clearly that the build was Modified or reconfigured;
  (b) You may not claim, or imply, that Modified results represent what Official Builds achieve.

9.3 **Your own marketing.** You may not use the Marks or results of Official Builds to market a Modified version as having WolOS performance, unless the claim is true for Your version and states it is Your version.

---

## 10. Notices and Attribution

10.1 **Keep notices.** You must keep, in every copy of a Covered File You Distribute or Deploy, all copyright, patent, trademark, license and attribution notices found in it, except notices that do not concern any part of the Covered Work.

10.2 **License text.** You must include a copy of this License (or a link to its canonical location, if a copy is impractical for the medium) with each copy of the Covered Work You Distribute.

10.3 **NOTICE file.** If the Covered Work includes a `NOTICE` file, You must include its readable contents with every copy You Distribute or Deploy, in at least one of: (a) a `NOTICE` file shipped with the work; (b) the Source Form or documentation; or (c) a display that the work generates, where such third-party notices normally appear. You may add Your own notices to the `NOTICE` file, set apart from the original text. You may not remove or alter the original text.

10.4 **No misleading notices.** You may not add notices that state or imply that the Licensor is responsible for Your Modifications.

---

## 11. Compatibility with Other Licenses

11.1 **Purpose.** This License is meant to coexist with the permissive and copyleft licenses used by Target Source and by other parts of WolOS.

11.2 **Permissive Target Source.** Covered patches may be applied to, and Distributed with, Target Source under permissive licenses (such as BSD, MIT or ISC). The Target Source license governs the Target Source; this License governs the Covered lines.

11.3 **GPL-licensed works.** Combining Covered Code with code under the GNU General Public License in a single program may create terms that cannot both be met. Such combination is not prohibited by this License, but You are responsible for meeting both licenses. The Licensor may, at her discretion, grant additional permissions in a file-level notice to allow such combinations.

11.4 **Secondary licenses.** The Licensor may mark a Covered File as also available under another named license. Where so marked, You may choose either.

11.5 **Conflict with law.** If a law, court order or regulation makes it impossible for You to comply with this License for some Covered Work, You must (a) comply to the maximum extent possible, and (b) describe the limitation and the affected code in the documentation required by Section 4.4.

---

## 12. Termination

12.1 **Automatic termination.** Your rights under this License terminate automatically if You fail to comply with any of its terms.

12.2 **Cure period.** If You cure the breach within thirty (30) days after You become aware of it, or after the Licensor or a Contributor notifies You of it, whichever is earlier, Your rights are reinstated as if they had never terminated.

12.3 **One reinstatement.** Section 12.2 applies only to Your first breach. After a second breach, Your rights are reinstated only if the Licensor expressly agrees in writing.

12.4 **Patents.** Termination under Section 3.2 is not subject to cure.

12.5 **Recipients keep their rights.** Termination of Your rights does not end the licenses of people who received the Covered Work from You, as long as they remain in compliance.

12.6 **Sublicenses and end users.** End-user licenses validly granted by You or Your distributors before termination survive it.

12.7 **Survival.** Sections 0, 1, 2.5, 3.2, 3.4, 4.2B, 6 (including 6.8 and 6.9), 7, 8, 12, 13, 14, 15, 17 and 19 survive termination, as do any obligations accrued before termination.

---

# Part III: Disclaimers and General Terms

## 13. No Warranty

13.1 THE COVERED WORK IS PROVIDED "AS IS" AND "AS AVAILABLE", WITH ALL FAULTS AND WITHOUT WARRANTY OF ANY KIND, EXPRESS, IMPLIED OR STATUTORY, INCLUDING ANY WARRANTY OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, TITLE, NON-INFRINGEMENT, ACCURACY, QUIET ENJOYMENT, OR THAT IT IS FREE OF DEFECTS, VIRUSES OR ERRORS.

13.2 KERNEL AND DRIVER CODE CAN CAUSE DATA LOSS, HARDWARE MALFUNCTION, OVERHEATING, SECURITY VULNERABILITIES, OR SYSTEM FAILURE. YOU ASSUME THE ENTIRE RISK OF USING, MODIFYING AND DEPLOYING THE COVERED WORK. THE COVERED WORK IS NOT DESIGNED OR INTENDED FOR USE IN SITUATIONS WHERE FAILURE COULD LEAD TO DEATH, PERSONAL INJURY, OR SEVERE PHYSICAL OR ENVIRONMENTAL DAMAGE.

13.3 THIS DISCLAIMER IS AN ESSENTIAL PART OF THIS LICENSE. NO USE OF THE COVERED WORK IS AUTHORIZED EXCEPT UNDER IT.

13.4 Where applicable law does not allow the exclusion of certain warranties, those warranties are limited to the minimum duration and scope that law permits.

---

## 14. Limitation of Liability

14.1 TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, IN NO EVENT AND UNDER NO LEGAL THEORY (CONTRACT, TORT INCLUDING NEGLIGENCE, STRICT LIABILITY OR OTHERWISE) WILL THE LICENSOR OR ANY CONTRIBUTOR BE LIABLE TO YOU OR ANY THIRD PARTY FOR ANY DIRECT, INDIRECT, SPECIAL, INCIDENTAL, CONSEQUENTIAL, EXEMPLARY OR PUNITIVE DAMAGES, INCLUDING LOSS OF PROFITS, REVENUE, DATA, GOODWILL, OR USE, COMPUTER FAILURE, OR BUSINESS INTERRUPTION, ARISING FROM OR RELATED TO THIS LICENSE OR THE COVERED WORK, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGES.

14.2 WHERE LIABILITY CANNOT BE EXCLUDED, THE TOTAL LIABILITY OF THE LICENSOR AND EACH CONTRIBUTOR IS LIMITED TO THE GREATER OF (A) THE AMOUNT YOU PAID THE LICENSOR FOR THE COVERED WORK, OR (B) ONE HUNDRED BRAZILIAN REAIS (BRL 100.00).

14.3 Nothing in this License limits liability for willful misconduct (dolo), for death or personal injury caused by gross negligence, or any liability that cannot be limited under applicable law, including mandatory consumer protection law.

---

## 15. Indemnity for Commercial Distributors

15.1 If You Deploy a Covered Work as part of a commercial product or service, You agree to defend, indemnify and hold harmless the Licensor and every Contributor against losses, damages, and costs (including reasonable attorney fees) arising from third-party claims caused by Your acts or omissions in that commercial Deployment, including any warranty or support You offered.

15.2 This obligation does not apply to claims that the unmodified Original Work infringes intellectual property rights.

15.3 The indemnified party must (a) notify You promptly in writing of the claim, (b) let You control the defense and settlement, and (c) cooperate reasonably at Your expense. You may not settle in a way that admits fault by, or imposes obligations on, the indemnified party without its consent.

---

## 16. Export and Sanctions Compliance

16.1 The Covered Work may be subject to export control and sanctions laws of Brazil, the United States, the European Union and other jurisdictions. You are responsible for complying with those laws when You Distribute or Deploy it.

16.2 This License grants no right to export or re-export the Covered Work in violation of any such law.

16.3 Nothing in this Section restricts publication of the Source Form where publicly available source is exempt under applicable law.

---

## 17. Governing Law and Jurisdiction

17.1 **Law.** This License is governed by the laws of the Federative Republic of Brazil, including Law No. 9,610/1998 (Copyright), Law No. 9,609/1998 (Software), and Law No. 9,279/1996 (Industrial Property), without regard to conflict-of-law rules. The United Nations Convention on Contracts for the International Sale of Goods does not apply.

17.2 **Courts.** Disputes arising from this License will be resolved by the courts of the Judicial District of the Capital of the State of São Paulo, Brazil, and the parties waive any other forum, however privileged. However, the Licensor may seek injunctive relief, or enforce a judgment, in any court with jurisdiction over You.

17.3 **Consumers.** If You are a consumer under a law that gives You the right to sue in Your home courts or under Your home law, this Section does not remove that right.

17.4 **Language.** This License is written in English. If the Licensor publishes a translation (including a Portuguese version), the Licensor may designate it as equally authoritative; otherwise the English text controls. Where a Brazilian court requires a Portuguese text, a sworn translation of the English text will be used.

---

## 18. Versions of This License

18.1 **New versions.** The Licensor may publish revised or new versions of this License. Each version has a distinguishing number. Only the Licensor may change the license terms; no one else may modify or publish a new version under this name.

18.2 **Choosing a version.** You may use a Covered Work under the version of this License it was received under, or, at Your option, under any later version published by the Licensor, unless the Covered Work's notice says "Version 1.0 only".

18.3 **Your own derivative license.** If You write a license based on this one, You must rename it, remove the Marks and the name "Wonderland" from it, and state that it differs from The Wonderland License.

---

## 19. General Terms

19.1 **Entire agreement.** This License is the entire agreement about the Covered Work between You and the Licensor regarding its subject matter, except where You and the Licensor have signed a separate written agreement, which then controls to the extent it conflicts.

19.2 **Severability.** If any provision is held invalid or unenforceable, it will be reformed to the minimum extent needed to make it enforceable, consistent with its intent, and the remaining provisions remain in full force.

19.3 **No waiver.** Failure to enforce a provision is not a waiver of it.

19.4 **Assignment.** The Licensor may assign her rights and obligations under this License. You may not assign Your rights except to a successor of Your entire business related to the Covered Work, who agrees to be bound by this License.

19.5 **Interpretation.** Headings and the summary at the top are for convenience only. "Including" means "including without limitation". Any rule that ambiguities are construed against the drafter does not apply.

19.6 **Notices.** Notices to the Licensor go to the contact address published in the `NOTICE` file of the Original Work. Notice to You is sufficient if sent to an address You published with Your Deployment, or posted where You publish its Source Form.

19.7 **Acceptance.** You are not required to accept this License to receive or possess a Covered Work. However, only this License gives You permission to modify, Distribute or Deploy it. By doing so, You indicate Your acceptance.

19.8 **Government users.** The Covered Work is a "commercial item" as defined in applicable government procurement rules, and government users acquire only the rights stated in this License.

---

## Appendix A: File Header Notices

### Source and patch files

```
SPDX-License-Identifier: LicenseRef-Wonderland-1.0
Copyright (c) <year> Julia Klee

This file is covered by The Wonderland License, Version 1.0.
You may use, modify and distribute it under that License.
See LICENSES/Wonderland-1.0.md or the NOTICE file.
```

For a patch file, put the notice in the patch description (above the first `---` or `diff` line), so it is not applied into the Target Source file.

### Modified files

Add below the original notice:

```
Modified <YYYY-MM-DD> by <Your name or entity>: <one-line description>.
```

---

## Appendix B: NOTICE File Template

```
WolOS
Copyright (c) <year> Julia Klee

This product includes works under The Wonderland License, Version 1.0
(LicenseRef-Wonderland-1.0). Source for covered files is available at:
<URL>

"WolOS", "Wonderland OS", "WolKernel", the WolOS logo and the WolOS
characters are marks of Julia Klee. This notice grants no right to use them.

Contact for licensing and permissions: <address>
```
