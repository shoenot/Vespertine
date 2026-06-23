
#ifndef _MLIBC_BSD_STDLIB_H
#define _MLIBC_BSD_STDLIB_H

#ifdef __cplusplus
extern "C" {
#endif

#include <mlibc-config.h>

#ifndef __MLIBC_ABI_ONLY

char *getbsize(int *__header_length, long *__block_size);
long long
strtonum(const char *__string, long long __minimum, long long __maximum, const char **__error);

#if defined(_DEFAULT_SOURCE)
int getloadavg(double *__loadavg, int __count);
#endif

#endif /* !__MLIBC_ABI_ONLY */

#ifdef __cplusplus
}
#endif

#endif /* _MLIBC_BSD_STDLIB_H */
