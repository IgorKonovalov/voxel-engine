vxe-tester @VERSION@ (commit @COMMIT@)
--------------------------------------

Thank you for testing the voxel engine. This archive holds two programs:

  vxe-tester    the launcher: a small window with a menu
  flythrough    the engine's test scene viewer, which the launcher starts for each run

Keep both in the same folder. Nothing is installed, and nothing is sent anywhere: you send
the report yourself (see "Sending the report").


Running it
----------

Linux:    unpack the archive, open a terminal in the folder and run ./vxe-tester
Windows:  unpack the zip (right-click, "Extract All"), open the folder and run vxe-tester.exe

Windows warns about an unrecognised app ("Windows protected your PC"), because the programs
are not code-signed. Click "More info", then "Run anyway".


The menu
--------

Arrow keys move and change, Enter chooses; or press a row's number.

  1 scene           cave (a streamed cave), showcase (a lit hall), castle
  2 mode            benchmark (a fixed flight, the same on every machine, then it closes;
                    the headlamp is off) or tour (the flight loops; move or look to fly
                    yourself; the headlamp is on in the cave and the castle)
  3 output          1280x720, 1920x1080, or your display's size
  4 internal scale  1, 1.5 or 2 (the picture is drawn smaller and upscaled)
  5 run             runs the scene in a second window
  6 run all benchmarks   the three scenes one after the other (about 3 minutes)
  7 save report     packs everything into one zip and shows where it is
  8 open report folder
  9 quit            (Esc does the same)

The best help: "run all benchmarks", then "save report", then send the zip. A free flight in
each scene ("tour", then move) tells us how it feels; say so in your message.

The run's window opens in front of the launcher (on some Linux desktops behind it; look for
a second window). The launcher shows "running" until the run ends.


Keys in a run
-------------

  W A S D            move           Space / C   up / down     Shift   faster
  mouse              look
  left click         dig (free flight only)
  T                  resume the tour after you flew yourself
  F1                 show or hide the help panel
  Esc                end the run (a benchmark ended early is marked "aborted")

A benchmark flies by itself; only Esc does anything there.


What the report holds
---------------------

The report is a folder named vxe-report-<version>-<os>-<date and time in UTC>, saved as a
zip next to it, under "reports" in this folder (or in your system's temporary folder if this
folder is not writable). The launcher's window shows both paths.

  system.json   your operating system and its version, the CPU model and its core count,
                the total memory, your displays' size and refresh rate, and every graphics
                adapter: its name, vendor and device numbers, driver and driver version, and
                the limits the engine asks of it
  tester.log    each command the launcher ran and how it ended
  runs/NN-<scene>-<mode>/log.txt    everything the run printed
  runs/NN-<scene>-<mode>/run.json   the run's frame times (median, 95th and 99th percentile,
                worst), the resolution, whether the headlamp was on, the adapter, the peak
                memory, and how it ended

It holds no user name, no computer name, no file paths from your home folder and no serial
numbers. Every file is plain text: open any of them before you send them.


Sending the report
------------------

Attach the vxe-report-....zip to an email or a message to the person who sent you this
archive. If a run failed, the launcher shows its last lines in red; the zip has the whole log.


What it needs
-------------

  A 64-bit x86 machine and a graphics card with up-to-date drivers.

  Linux:    a Vulkan driver (Mesa's RADV or ANV, or NVIDIA's) and the Vulkan loader
            (libvulkan.so.1, package "libvulkan1" or "vulkan-icd-loader"); ALSA (libasound.so.2)
            and udev (libudev.so.1), which every desktop has; Wayland or X11. The programs
            need glibc @GLIBC_FLOOR@ or later. Release archives are built on Ubuntu 22.04,
            so Ubuntu 22.04, Debian 12, Fedora 36 and newer releases run them.
  Windows:  Windows 10 or 11 with a DirectX 12 or Vulkan driver.

Licences: the engine is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE); the libraries it
uses are listed with their licences in THIRD-PARTY-NOTICES.txt.
