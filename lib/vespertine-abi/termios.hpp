#include <stdint.h>

// cflags
constexpr static const uintptr_t CSIZE	=	0o0000060;
constexpr static const uintptr_t CS5    =	0o0000000;
constexpr static const uintptr_t CS6    =	0o0000020;
constexpr static const uintptr_t CS7    =	0o0000040;
constexpr static const uintptr_t CS8    =	0o0000060;
constexpr static const uintptr_t CSTOPB	=	0o0000100;
constexpr static const uintptr_t CREAD	=	0o0000200;
constexpr static const uintptr_t PARENB	=	0o0000400;
constexpr static const uintptr_t PARODD	=	0o0001000;
constexpr static const uintptr_t HUPCL	=	0o0002000;
constexpr static const uintptr_t CLOCAL	=	0o0004000;

// iflags
constexpr static const uintptr_t IGNBRK	=	0o0000001;  /* Ignore break condition.  */
constexpr static const uintptr_t BRKINT	=	0o0000002;  /* Signal interrupt on break.  */
constexpr static const uintptr_t IGNPAR	=	0o0000004;  /* Ignore characters with parity errors.  */
constexpr static const uintptr_t PARMRK	=	0o0000010;  /* Mark parity and framing errors.  */
constexpr static const uintptr_t INPCK	=	0o0000020;  /* Enable input parity check.  */
constexpr static const uintptr_t ISTRIP	=	0o0000040;  /* Strip 8th bit off characters.  */
constexpr static const uintptr_t INLCR	=	0o0000100;  /* Map NL to CR on input.  */
constexpr static const uintptr_t IGNCR	=	0o0000200;  /* Ignore CR.  */
constexpr static const uintptr_t ICRNL	=	0o0000400;  /* Map CR to NL on input.  */
constexpr static const uintptr_t IUCLC	=	0o0001000;  /* Map uppercase characters to lowercase on input (not in POSIX).  */
constexpr static const uintptr_t IXON	=	0o0002000;  /* Enable start/stop output control.  */
constexpr static const uintptr_t IXANY	=	0o0004000;  /* Enable any character to restart output.  */
constexpr static const uintptr_t IXOFF	=	0o0010000;  /* Enable start/stop input control.  */
constexpr static const uintptr_t IMAXBEL=	0o0020000;  /* Ring bell when input queue is full (not in POSIX).  */
constexpr static const uintptr_t IUTF8	=	0o0040000;  /* Input is UTF8 (not in POSIX).  */

// lflags
constexpr static const uintptr_t ISIG	=	0o0000001;   /* Enable signals.  */
constexpr static const uintptr_t ICANON	=	0o0000002;   /* Canonical input (erase and kill processing).  */
constexpr static const uintptr_t ECHO	=	0o0000010;   /* Enable echo.  */
constexpr static const uintptr_t ECHOE	=	0o0000020;   /* Echo erase character as error-correcting backspace.  */
constexpr static const uintptr_t ECHOK	=	0o0000040;   /* Echo KILL.  */
constexpr static const uintptr_t ECHONL	=	0o0000100;   /* Echo NL.  */
constexpr static const uintptr_t NOFLSH	=	0o0000200;   /* Disable flush after interrupt or quit.  */
constexpr static const uintptr_t TOSTOP	=	0o0000400;   /* Send SIGTTOU for background output.  */

// oflags
constexpr static const uintptr_t OPOST	=	0o0000001;  /* Post-process output.  */
constexpr static const uintptr_t OLCUC	=	0o0000002;  /* Map lowercase characters to uppercase on output. (not in POSIX).  */
constexpr static const uintptr_t ONLCR	=	0o0000004;  /* Map NL to CR-NL on output.  */
constexpr static const uintptr_t OCRNL	=	0o0000010;  /* Map CR to NL on output.  */
constexpr static const uintptr_t ONOCR	=	0o0000020;  /* No CR output at column 0.  */
constexpr static const uintptr_t ONLRET	=	0o0000040;  /* NL performs CR function.  */
constexpr static const uintptr_t OFILL	=	0o0000100;  /* Use fill characters for delay.  */
constexpr static const uintptr_t OFDEL	=	0o0000200;  /* Fill is DEL.  */
constexpr static const uintptr_t VTDLY	=	0o0040000;  /* Select vertical-tab delays:  */
constexpr static const uintptr_t VT0  	=	0o0000000;  /* Vertical-tab delay type 0.  */
constexpr static const uintptr_t VT1  	=	0o0040000;  /* Vertical-tab delay type 1.  */

// c_cc 
constexpr static const uintptr_t VINTR     =  0;
constexpr static const uintptr_t VQUIT     =  1;
constexpr static const uintptr_t VERASE    =  2;
constexpr static const uintptr_t VKILL     =  3;
constexpr static const uintptr_t VEOF      =  4;
constexpr static const uintptr_t VTIME     =  5;
constexpr static const uintptr_t VMIN      =  6;
constexpr static const uintptr_t VSWTC     =  7;
constexpr static const uintptr_t VSTART    =  8;
constexpr static const uintptr_t VSTOP     =  9;
constexpr static const uintptr_t VSUSP     = 10;
constexpr static const uintptr_t VEOL      = 11;
constexpr static const uintptr_t VREPRINT  = 12;
constexpr static const uintptr_t VDISCARD  = 13;
constexpr static const uintptr_t VWERASE   = 14;
constexpr static const uintptr_t VLNEXT    = 15;
constexpr static const uintptr_t VEOL2     = 16;

// baud rates 
/* POSIX required baud rates */
constexpr static const uintptr_t B0         =		      0;		/* Hang up or ispeed == ospeed */
constexpr static const uintptr_t B50        =		     50;
constexpr static const uintptr_t B75        =		     75;
constexpr static const uintptr_t B110       =		    110;
constexpr static const uintptr_t B134       =		    134;		/* Really 134.5 baud by POSIX spec */
constexpr static const uintptr_t B150       =		    150;
constexpr static const uintptr_t B200       =		    200;
constexpr static const uintptr_t B300       =		    300;
constexpr static const uintptr_t B600       =		    600;
constexpr static const uintptr_t B1200      =		   1200;
constexpr static const uintptr_t B1800      =		   1800;
constexpr static const uintptr_t B2400      =		   2400;
constexpr static const uintptr_t B4800      =		   4800;
constexpr static const uintptr_t B9600      =		   9600;
constexpr static const uintptr_t B19200     =		  19200;
constexpr static const uintptr_t B38400 	=         38400;

/* Other baud rates, "nonstandard" but known to be used */
constexpr static const uintptr_t B7200      =		   7200;
constexpr static const uintptr_t B14400     =	      14400;
constexpr static const uintptr_t B28800     =	      28800;
constexpr static const uintptr_t B33600     =	      33600;
constexpr static const uintptr_t B57600     =	      57600;
constexpr static const uintptr_t B76800     =	      76800;
constexpr static const uintptr_t B115200    =	     115200;
constexpr static const uintptr_t B153600    =	     153600;
constexpr static const uintptr_t B230400    =	     230400;
constexpr static const uintptr_t B307200    =	     307200;
constexpr static const uintptr_t B460800    =	     460800;
constexpr static const uintptr_t B500000    =	     500000;
constexpr static const uintptr_t B576000    =	     576000;
constexpr static const uintptr_t B614400    =	     614400;
constexpr static const uintptr_t B921600    =	     921600;
constexpr static const uintptr_t B1000000	=       1000000;
constexpr static const uintptr_t B1152000	=       1152000;
constexpr static const uintptr_t B1500000	=       1500000;
constexpr static const uintptr_t B2000000	=       2000000;
constexpr static const uintptr_t B2500000	=       2500000;
constexpr static const uintptr_t B3000000	=       3000000;
constexpr static const uintptr_t B3500000	=       3500000;
constexpr static const uintptr_t B4000000	=       4000000;
constexpr static const uintptr_t B5000000	=       5000000;
constexpr static const uintptr_t B10000000	=      10000000;

struct Termios {
  uint32_t  c_iflag;
  uint32_t  c_oflag;
  uint32_t  c_cflag;
  uint32_t  c_lflag;
  uint8_t    c_line;
  uint8_t  c_cc[32];
  uint32_t c_ispeed;
  uint32_t c_ospeed;
};

struct TermCommand {
  enum class Tag {
    TermCommand_SetTermios,
    TermCommand_GetTermios,
    TermCommand_GetWindowSize,
  };

  struct TermCommand_SetTermios_Body {
    Termios _0;
  };

  Tag tag;
  union {
    TermCommand_SetTermios_Body set_termios;
  };
};
