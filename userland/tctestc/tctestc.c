#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <termios.h>

// Store original settings to restore them on exit
struct termios orig_termios;

void print_termios(const char *label, struct termios *t) {
    printf("%s - iflag: 0x%X, oflag: 0x%X, cflag: 0x%X, lflag: 0x%X\n\r", 
           label, t->c_iflag, t->c_oflag, t->c_cflag, t->c_lflag);
}

void disableRawMode() {
    // Restore the terminal to its original state
    tcsetattr(STDIN_FILENO, TCSAFLUSH, &orig_termios);
}

void enableRawMode() {
    // Get current terminal attributes
    tcgetattr(STDIN_FILENO, &orig_termios);
    print_termios("Original", &orig_termios);
    
    // Register the cleanup function to run automatically when the program exits
    atexit(disableRawMode);

    struct termios raw = orig_termios;
    
    // Disable canonical mode (line buffering) and local echo
    raw.c_lflag &= ~(ICANON | ECHO);
    
    // Set read timeouts: read() returns as soon as 1 byte is available
    raw.c_cc[VMIN] = 1;
    raw.c_cc[VTIME] = 0;

    print_termios("Setting Raw", &raw);

    // Apply the new raw settings immediately
    tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw);

    struct termios verified;
    tcgetattr(STDIN_FILENO, &verified);
    print_termios("Verified", &verified);
}

int main() {
    setvbuf(stdout, NULL, _IONBF, 0);
    enableRawMode();

    printf("Raw mode enabled. Type anything to see hex codes (Press 'q' to quit):\n\r");
    char c;
    // read() returns 0 on EOF, -1 on error, or the number of bytes read
    while (read(STDIN_FILENO, &c, 1) == 1 && c != 'q') {
        // Print control characters cleanly, mapping carriage returns for readability
        if (c == '\r' || c == '\n') {
            printf("Hex: 0x%02X (Enter/Return)\n\r", c);
        } else if (c < 32 || c == 127) {
            printf("Hex: 0x%02X (Control Char)\n\r", c);
        } else {
            printf("Hex: 0x%02X ('%c')\n\r", c, c);
        }
    }

    printf("Exiting and restoring terminal...\n");
    return 0;
}
