ENTRY(_start)

SECTIONS
{
    . = 0x40080000;
    
    .text : {
        KEEP(*(.text.entrypoint))
        *(.text .text.*)
        *(.rodata .rodata.*)
        
        . = ALIGN(16);
    }
    
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }
    
    .bss : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }
    
    _end = .;
}