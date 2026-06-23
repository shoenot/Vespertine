#ifndef _SYS_SYSMACROS_H
#define _SYS_SYSMACROS_H

#ifdef __cplusplus
extern "C" {
#endif

static __inline__ unsigned int __mlibc_dev_major(unsigned long long device) {
	return ((device >> 8) & 0xfff) | ((unsigned int)(device >> 32) & ~0xfff);
}

static __inline__ unsigned int __mlibc_dev_minor(unsigned long long device) {
	return (device & 0xff) | ((unsigned int)(device >> 12) & ~0xff);
}

static __inline__ unsigned long long __mlibc_dev_makedev(unsigned int major, unsigned int minor) {
	return (minor & 0xff) | ((major & 0xfff) << 8) | ((unsigned long long)(minor & ~0xff) << 12)
	       | ((unsigned long long)(major & ~0xfff) << 32);
}

#define major(device) __mlibc_dev_major(device)
#define minor(device) __mlibc_dev_minor(device)
#define makedev(major, minor) __mlibc_dev_makedev(major, minor)

#ifdef __cplusplus
}
#endif

#endif /* _SYS_SYSMACROS_H */
