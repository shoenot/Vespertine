#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>

#define BUFFER_SIZE 4096

void concat(int src_fd) {
    char buffer[BUFFER_SIZE];
    ssize_t bytes_read;

    while ((bytes_read = read(src_fd, buffer, BUFFER_SIZE)) > 0) {
        ssize_t bytes_written = 0;
        while (bytes_written < bytes_read) {
            ssize_t res = write(STDOUT_FILENO, buffer + bytes_written, bytes_read - bytes_written);
            if (res < 0) {
                perror("Error writing to stdout");
                exit(EXIT_FAILURE);
            }
            bytes_written += res;
        }
    }

    if (bytes_read < 0) {
        perror("Error reading file");
        exit(EXIT_FAILURE);
    }
}

int main(int argc, char *argv[]) {
    // If no arguments, read from standard input
    if (argc == 1) {
        concat(STDIN_FILENO);
    } else {
        // Loop through all file arguments
        for (int i = 1; i < argc; i++) {
            int fd = open(argv[i], O_RDONLY);
            if (fd < 0) {
                perror(argv[i]);
                continue; // Move to the next file even if one fails
            }
            concat(fd);
            close(fd);
        }
    }

    return EXIT_SUCCESS;
}
