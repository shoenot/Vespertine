
// cflags
pub static CSIZE:	u32 =	0o0000060;
pub static CS5:     u32 =	0o0000000;
pub static CS6:     u32 =	0o0000020;
pub static CS7:     u32 =	0o0000040;
pub static CS8:     u32 =	0o0000060;
pub static CSTOPB:	u32 =	0o0000100;
pub static CREAD:	u32 =	0o0000200;
pub static PARENB:	u32 =	0o0000400;
pub static PARODD:	u32 =	0o0001000;
pub static HUPCL:	u32 =	0o0002000;
pub static CLOCAL:	u32 =	0o0004000;

// iflags
pub static IGNBRK:	u32 =	0o0000001;  /* Ignore break condition.  */
pub static BRKINT:	u32 =	0o0000002;  /* Signal interrupt on break.  */
pub static IGNPAR:	u32 =	0o0000004;  /* Ignore characters with parity errors.  */
pub static PARMRK:	u32 =	0o0000010;  /* Mark parity and framing errors.  */
pub static INPCK:	u32 =	0o0000020;  /* Enable input parity check.  */
pub static ISTRIP:	u32 =	0o0000040;  /* Strip 8th bit off characters.  */
pub static INLCR:	u32 =	0o0000100;  /* Map NL to CR on input.  */
pub static IGNCR:	u32 =	0o0000200;  /* Ignore CR.  */
pub static ICRNL:	u32 =	0o0000400;  /* Map CR to NL on input.  */
pub static IUCLC:	u32 =	0o0001000;  /* Map uppercase characters to lowercase on input (not in POSIX).  */
pub static IXON:	u32 =	0o0002000;  /* Enable start/stop output control.  */
pub static IXANY:	u32 =	0o0004000;  /* Enable any character to restart output.  */
pub static IXOFF:	u32 =	0o0010000;  /* Enable start/stop input control.  */
pub static IMAXBEL:	u32 =	0o0020000;  /* Ring bell when input queue is full (not in POSIX).  */
pub static IUTF8:	u32 =	0o0040000;  /* Input is UTF8 (not in POSIX).  */

// lflags
pub static ISIG:	u32 =	0o0000001;   /* Enable signals.  */
pub static ICANON:	u32 =	0o0000002;   /* Canonical input (erase and kill processing).  */
pub static ECHO:	u32 =	0o0000010;   /* Enable echo.  */
pub static ECHOE:	u32 =	0o0000020;   /* Echo erase character as error-correcting backspace.  */
pub static ECHOK:	u32 =	0o0000040;   /* Echo KILL.  */
pub static ECHONL:	u32 =	0o0000100;   /* Echo NL.  */
pub static NOFLSH:	u32 =	0o0000200;   /* Disable flush after interrupt or quit.  */
pub static TOSTOP:	u32 =	0o0000400;   /* Send SIGTTOU for background output.  */

// oflags
pub static OPOST:	u32 =	0o0000001;  /* Post-process output.  */
pub static OLCUC:	u32 =	0o0000002;  /* Map lowercase characters to uppercase on output. (not in POSIX).  */
pub static ONLCR:	u32 =	0o0000004;  /* Map NL to CR-NL on output.  */
pub static OCRNL:	u32 =	0o0000010;  /* Map CR to NL on output.  */
pub static ONOCR:	u32 =	0o0000020;  /* No CR output at column 0.  */
pub static ONLRET:	u32 =	0o0000040;  /* NL performs CR function.  */
pub static OFILL:	u32 =	0o0000100;  /* Use fill characters for delay.  */
pub static OFDEL:	u32 =	0o0000200;  /* Fill is DEL.  */
pub static VTDLY:	u32 =	0o0040000;  /* Select vertical-tab delays:  */
pub static VT0: 	u32 =	0o0000000;  /* Vertical-tab delay type 0.  */
pub static VT1: 	u32 =	0o0040000;  /* Vertical-tab delay type 1.  */

// c_cc 
pub static VINTR:       usize =  0;
pub static VQUIT:       usize =  1;
pub static VERASE:      usize =  2;
pub static VKILL:       usize =  3;
pub static VEOF:        usize =  4;
pub static VTIME:       usize =  5;
pub static VMIN:        usize =  6;
pub static VSWTC:       usize =  7;
pub static VSTART:      usize =  8;
pub static VSTOP:       usize =  9;
pub static VSUSP:       usize = 10;
pub static VEOL:        usize = 11;
pub static VREPRINT:	usize = 12;
pub static VDISCARD:	usize = 13;
pub static VWERASE:     usize = 14;
pub static VLNEXT:      usize = 15;
pub static VEOL2:       usize = 16;

// baud rates 
/* POSIX required baud rates */
pub static B0:      u32 =		     0;		/* Hang up or ispeed == ospeed */
pub static B50:     u32 =		    50;
pub static B75:     u32 =		    75;
pub static B110:	u32 =		   110;
pub static B134:	u32 =		   134;		/* Really 134.5 baud by POSIX spec */
pub static B150:	u32 =		   150;
pub static B200:	u32 =		   200;
pub static B300:	u32 =		   300;
pub static B600:	u32 =		   600;
pub static B1200:	u32 =		  1200;
pub static B1800:	u32 =		  1800;
pub static B2400:	u32 =		  2400;
pub static B4800:	u32 =		  4800;
pub static B9600:	u32 =		  9600;
pub static B19200: u32 =		 19200;
pub static B38400: u32	=        38400;

/* Other baud rates, "nonstandard" but known to be used */
pub static B7200:       u32 =		   7200;
pub static B14400:      u32 =	      14400;
pub static B28800:      u32 =	      28800;
pub static B33600:      u32 =	      33600;
pub static B57600:      u32 =	      57600;
pub static B76800:      u32 =	      76800;
pub static B115200:     u32 =	     115200;
pub static B153600:     u32 =	     153600;
pub static B230400:     u32 =	     230400;
pub static B307200:     u32 =	     307200;
pub static B460800:     u32 =	     460800;
pub static B500000:     u32 =	     500000;
pub static B576000:     u32 =	     576000;
pub static B614400:     u32 =	     614400;
pub static B921600:     u32 =	     921600;
pub static B1000000:	u32 =       1000000;
pub static B1152000:	u32 =       1152000;
pub static B1500000:	u32 =       1500000;
pub static B2000000:	u32 =       2000000;
pub static B2500000:	u32 =       2500000;
pub static B3000000:	u32 =       3000000;
pub static B3500000:	u32 =       3500000;
pub static B4000000:	u32 =       4000000;
pub static B5000000:	u32 =       5000000;
pub static B10000000:	u32 =      10000000;
