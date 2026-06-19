PROG := grep
SRCS := binary.c file.c grep.c mmfile.c queue.c util.c
OBJS := $(SRCS:.c=.o)

CFLAGS += -Wall
LDLIBS += -lz

.PHONY: all clean

all: $(PROG)

$(PROG): $(OBJS)
	$(CC) $(LDFLAGS) -o $@ $(OBJS) $(LDLIBS)

%.o: %.c grep.h
	$(CC) $(CPPFLAGS) $(CFLAGS) -c -o $@ $<

clean:
	rm -f $(PROG) $(OBJS)
