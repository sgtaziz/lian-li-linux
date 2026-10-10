# RX-78 ICE Community

Install from the Template Browser to bind the CPU/GPU/RAM sensor categories to sensors available on your PC. Direct JSON import requires choosing the sensors manually in the editor.

CPU temperature, GPU temperature, GPU load, and the GPU load gauge use intentionally unresolved hwmon sources as their initial bindings. Category resolution replaces them when a matching sensor is found. If none is found, the numeric readout remains blank and the GPU ring remains unfilled; CPU usage is never substituted for a temperature or GPU measurement.

An unfilled ring without a numeric load readout means the GPU load sensor is unavailable, rather than a measured 0% load. Select a suitable sensor in the template editor if automatic binding does not find one. These placeholders apply at installation; they do not change the daemon's handling of sensors that disappear later.

See CREDITS.md and the bundled notices for the assets' licenses.
