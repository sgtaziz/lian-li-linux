# RX-78 / Flight Enhanced

Animated 1920×480 horizontal dashboard for the Universal Screen 8.8-inch. A separate catalog variant of RX-78 ICE, preserving the original template.

CPU/GPU utilization is the main numerical readout, with temperatures below. LOAD text is white through 75%, amber above 75% through 90%, and red above 90%. The right-hand panel shows RX/download and TX/upload in decimal MB/s with directional arrows. Network readings count all traffic on the chosen interface, not just Internet transfers. Review network bindings after changing Wi-Fi, Ethernet or VPN connections.

All six sensor widgets use portable sensor categories. Install through Browse Online Templates and confirm the detected sources. Installation clears each category marker when assigning its source; unavailable categories retain their marker and display N/A until selected in the editor. The neutral sources in the portable JSON are placeholders, not actual CPU/GPU temperature or usage bindings.

The head illustration is outside the panels on the left. White panel interiors have 20% opacity and a subtle bevel. A small top-down booster-inspired jet sits beside the right-aligned clock, with a fading static trail. Twinkling stars and gently pulsing blue engine exhaust share one 12-second, 1920×480 H.264 background video at 8 fps; the jet and trail remain stationary. FFmpeg must be available in the daemon environment. The animated background uses more resources than the original static template; no performance benchmark is claimed.

The local layout, animation and sensor display were approved by @marvinduranr on a Universal Screen 8.8-inch in horizontal 1920×480 LCD mode, using Lian Li Linux 1.2.6 in Fedora 44 Distrobox on Bazzite with an AMD RX 9070 XT. Full installation of this new variant through the upstream catalog remains untested until publication. The preview uses simulated sensor readings.

This is unofficial, noncommercial fan work. Retain the per-asset credits, CC BY-NC 3.0 text, and the separate Amuro font license notices. See CREDITS.md.
