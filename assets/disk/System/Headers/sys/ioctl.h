#ifndef _SYS_IOCTL_H
#define _SYS_IOCTL_H

#include <abi-bits/ioctls.h>
#include <mlibc-config.h>

/* On Linux, sys/ioctl.h includes the termios ioctls. */
#if __MLIBC_LINUX_OPTION
#include <asm/ioctls.h>
#include <sys/ttydefaults.h>
#endif

/* BSD applications also expect struct winsize from sys/ioctl.h. */
#if __MLIBC_LINUX_OPTION || __MLIBC_BSD_OPTION
#	include <bits/winsize.h>
#endif

#ifdef __cplusplus
extern "C" {
#endif

#ifndef __MLIBC_ABI_ONLY

int ioctl(int __fd, unsigned long __request, ...);

#endif /* !__MLIBC_ABI_ONLY */

#ifdef __cplusplus
}
#endif

#endif /* _SYS_IOCTL_H */
