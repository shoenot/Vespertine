#include <fcntl.h>
#include <unistd.h>
#include <string.h>

int main(void) {
    int fd = open("Docs/partial.bin", O_RDWR | O_CREAT | O_TRUNC, 0644);
    
    char data[1024];
    memset(data, 'X', sizeof(data));
    write(fd, data, sizeof(data));
    
    ftruncate(fd, 100);
    ftruncate(fd, 1024);
    lseek(fd, 100, SEEK_SET);
    
    char check[924];
    read(fd, check, sizeof(check));
    
    for (int i = 0; i < sizeof(check); i++)
        if (check[i] != 0)
            return 1;

    return 0;
}
