#include <stdio.h>
#include <stdint.h>

uint32_t process_bits(uint32_t val) {
    // Manual 32-bit byte swap (endianness conversion)
    uint32_t swapped = ((val >> 24) & 0x000000FF) |
                       ((val >> 8)  & 0x0000FF00) |
                       ((val << 8)  & 0x00FF0000) |
                       ((val << 24) & 0xFF000000);
    
    // Intensive bitwise mixing
    uint32_t x = swapped ^ 0xDEADBEEF;
    x = (x << 13) | (x >> 19);
    x = ~x & 0x55555555;
    x = (x ^ (x >> 3)) + (x << 5);
    x ^= (swapped & 0x0F0F0F0F);
    x = (x << 7) ^ (x >> 25);
    
    return x;
}

int main(void) {
    uint32_t acc = 0;
    
    // Execute a large volume of operations to stress test compilation/optimization
    for (uint32_t i = 0; i < 5000000; i++) {
        acc += process_bits(i ^ acc);
    }
    
    printf("Checksum Result: 0x%08X\n", acc);
    return 0;
}
