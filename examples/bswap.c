#include <stdio.h>
#include <limits.h>

int main(unsigned argc, char** argv) {
	unsigned bswap_argc = ((argc << 24) & 0xff000000) | ((argc << 8) & 0x00ff0000) | ((argc >> 8) & 0xff00) | ((argc >> 24) & 0xff);
	printf("%08x %08x\n", argc, bswap_argc);
}
