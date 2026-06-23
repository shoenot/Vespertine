#ifndef _MLIBC_BSD_GRP_H
#define _MLIBC_BSD_GRP_H

#ifdef __cplusplus
extern "C" {
#endif

#ifndef __MLIBC_ABI_ONLY

const char *group_from_gid(gid_t __gid, int __nogroup);

#endif /* !__MLIBC_ABI_ONLY */

#ifdef __cplusplus
}
#endif

#endif /* _MLIBC_BSD_GRP_H */
