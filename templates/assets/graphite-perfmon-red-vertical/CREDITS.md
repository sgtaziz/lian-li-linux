# Graphite Perfmon Red (Vertical)

For the Universal Screen 8.8" (vertical): CPU and GPU temperature and load, memory and NVMe temperature, each with a gauge and a 60-second history graph.

- Background art: generated procedurally for this template, no third-party imagery.
- Fonts: BarlowSemiCondensed, SIL Open Font License 1.1 (see the OFL-*.txt files).
- NVMe widgets bind to the hwmon `nvme` / `Composite` sensor directly (no sensor_category), so they read 0 on systems without an NVMe drive.
