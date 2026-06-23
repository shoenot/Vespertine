#ifndef _MLIBC_BSD_ERR_H
#define _MLIBC_BSD_ERR_H

#include <stdarg.h>

#ifdef __cplusplus
extern "C" {
#endif

void warnc(int, const char *, ...)
	__attribute__((__format__(__printf__, 2, 3)));
void vwarnc(int, const char *, va_list)
	__attribute__((__format__(__printf__, 2, 0)));

void errc(int, int, const char *, ...)
	__attribute__((__noreturn__, __format__(__printf__, 3, 4)));
void verrc(int, int, const char *, va_list)
	__attribute__((__noreturn__, __format__(__printf__, 3, 0)));

#ifdef __cplusplus
}
#endif

#endif
