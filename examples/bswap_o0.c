
undefined8 main(uint param_1)

{
  printf("%08x %08x\n",(ulong)param_1,
         (ulong)(param_1 >> 0x18 | param_1 << 0x18 | (param_1 & 0xff00) << 8 | param_1 >> 8 & 0xff00
                ));
  return 0;
}

