#ifndef _MLIBC_BSD_PWD_H
#define _MLIBC_BSD_PWD_H

#ifdef __cplusplus
extern "C" {
#endif

#ifndef __MLIBC_ABI_ONLY

const char *user_from_uid(uid_t __uid, int __nouser);

#endif /* !__MLIBC_ABI_ONLY */

#ifdef __cplusplus
}
#endif

#endif /* _MLIBC_BSD_PWD_H */
