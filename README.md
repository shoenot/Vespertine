# Vespertine

Vespertine is an experimental operating system being developed to explore the possibilities of a capability based, object-oriented operating system. 
The end goal is to provide modern semantics that allow for an ergonomic user experience, with proper permissions management, sandboxing, and all that jazz.  
  
The kernel is hybrid in nature. Drivers currently all run in ring 0, but the plan is to move everything except the performance critical drivers into userspace eventually. 
The ABI and everything is still very volatile so there is no documentation just yet, but that is a definite priority as soon as some things are solidified. 

## Compiling and running
Vespertine currently only has a virtio block driver, and relies on QEMU to run. If you want to try it out, simply clone the repository and run 

```bash
make run 
```

to build the disk image and launch it in QEMU. You can adjust the makefile for cores and memory and other options if necessary. 

## Credits
Managarm team: for providing [mlibc](https://github.com/managarm/mlibc) and also for the fact that my ext2 driver started off based on their code.  

## Third-Party Licenses

This project includes third-party assets subject to their own license terms:

### Linux Console Fonts (.psf)
* **Author:** Copyright © 2004–24, John Zaitseff.
* **License:** GNU General Public License (GPL) version 3.0 or later.
* **Source:** Distributed as part of the Linux Console Fonts package.

As the font package does not ship with a standalone license file, the author's official distribution terms are reproduced below:

> "The Linux Console Fonts package is free software that is distributed under the terms of the GNU General Public License. You can redistribute it and/or modify it under the terms of that License as published by the Free Software Foundation, either version 3 or (at your option) any later version. This font package is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE."

The full text of the GNU General Public License v3.0 can be found online at: https://www.gnu.org/licenses/gpl-3.0.txt
