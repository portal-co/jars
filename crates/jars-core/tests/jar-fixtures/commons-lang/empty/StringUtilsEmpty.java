package app;

import org.apache.commons.lang3.StringUtils;

public final class StringUtilsEmpty {
    public static void run() {
        System.out.println(StringUtils.isEmpty(null));
        System.out.println(StringUtils.isEmpty(""));
        System.out.println(StringUtils.isEmpty(" "));
        System.out.println(StringUtils.isNotEmpty(null));
        System.out.println(StringUtils.isNotEmpty("jars"));
    }


    public static void main(String[] args) {
        run();
    }
}
