#ifndef _MLIBC_BSD_STRING_H
#define _MLIBC_BSD_STRING_H

#include <abi-bits/mode_t.h>

#ifdef __cplusplus
extern "C" {
#endif

#ifndef __MLIBC_ABI_ONLY

void strmode(mode_t __mode, char *__buffer);

#endif /* !__MLIBC_ABI_ONLY */

#ifdef __cplusplus
}
#endif

#endif /* _MLIBC_BSD_STRING_H */
