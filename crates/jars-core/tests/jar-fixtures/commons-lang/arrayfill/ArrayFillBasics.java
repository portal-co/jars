package app;

import org.apache.commons.lang3.ArrayFill;

public final class ArrayFillBasics {
    public static void run() {
        byte[] filled = ArrayFill.fill(new byte[4], (byte) 7);
        for (byte value : filled) {
            System.out.println((int) value);
        }
        byte[] negative = ArrayFill.fill(new byte[2], (byte) -3);
        System.out.println((int) negative[0]);
        System.out.println((int) negative[1]);
    }


    public static void main(String[] args) {
        run();
    }
}
