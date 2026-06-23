#ifndef _UTIL_H
#define _UTIL_H

#include <mlibc-config.h>

#ifdef __cplusplus
extern "C" {
#endif

#define FMT_SCALED_STRSIZE 7

#ifndef __MLIBC_ABI_ONLY

int fmt_scaled(long long __number, char *__result);

#endif /* !__MLIBC_ABI_ONLY */

#ifdef __cplusplus
}
#endif

#endif /* _UTIL_H */
