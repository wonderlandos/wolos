# 0001 - Fork and tune mature software, not a new stack
Status: accepted
Context: IaraOS rebuilt compositor, toolkit and daemons from zero; too costly to finish.
Decision: FreeBSD 15.1 base + IaraOS kernel patches. Compositor = tuned Hyprland fork; IaraOS compositor pure logic (anim, titlebar, placement, consent, QoS) reused via C ABI in a Hyprland plugin. Shell = Rust GTK4 layer-shell port of the IaraOS shell look. System daemons/CLI = Rust port of IaraOS (iarad, admind, login, authcheck, par). Apps = mature ports, themed.
Consequences: custom code limited to shell look, glue and policy; Hyprland config syntax tracks the forked version.
