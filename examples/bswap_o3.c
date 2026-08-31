
undefined8 main(uint param_1)

{
  __printf_chk(2,"%08x %08x\n",param_1,
               param_1 >> 0x18 | (param_1 & 0xff0000) >> 8 | (param_1 & 0xff00) << 8 |
               param_1 << 0x18);
  return 0;
}

