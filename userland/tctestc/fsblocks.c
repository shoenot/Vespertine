#include <string.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#define TEST_SIZE (13 * 1024)
#define CHUNK 1024

static unsigned char expected(size_t offset) {
    return (unsigned char)((offset * 31 + 7) & 0xff);
}

int main(int argc, char **argv) {
    const char *path = "Docs/block-test.bin";

    if (argc != 2) {
        printf("usage: fsblocks write|verify|truncate|empty\n");
        return 1;
    }

    if (!strcmp(argv[1], "write")) {
        int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        unsigned char buf[CHUNK];

        for (size_t off = 0; off < TEST_SIZE; off += CHUNK) {
            for (size_t i = 0; i < CHUNK; i++)
                buf[i] = expected(off + i);

            if (write(fd, buf, CHUNK) != CHUNK)
                return 2;
        }

        close(fd);
        printf("write passed\n");
    } else if (!strcmp(argv[1], "verify")) {
        int fd = open(path, O_RDONLY);
        unsigned char buf[CHUNK];

        for (size_t off = 0; off < TEST_SIZE; off += CHUNK) {
            if (read(fd, buf, CHUNK) != CHUNK)
                return 3;

            for (size_t i = 0; i < CHUNK; i++)
                if (buf[i] != expected(off + i))
                    return 4;
        }

        close(fd);
        printf("verify passed\n");
    } else if (!strcmp(argv[1], "truncate")) {
        int fd = open(path, O_WRONLY);
        if (ftruncate(fd, 0))
            return 5;
        close(fd);
        printf("truncate passed\n");
    } else if (!strcmp(argv[1], "empty")) {
        int fd = open(path, O_RDONLY);
        unsigned char byte;
        if (read(fd, &byte, 1) != 0)
            return 6;
        close(fd);
        printf("empty passed\n");
    }

    return 0;
}
