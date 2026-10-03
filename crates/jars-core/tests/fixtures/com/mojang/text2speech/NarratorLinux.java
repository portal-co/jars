package com.mojang.text2speech;

public class NarratorLinux {
    public interface FliteLibrary {
        interface CmuUsKal16 {
            com.sun.jna.Pointer register_cmu_us_kal16(String name);
        }
    }
}
