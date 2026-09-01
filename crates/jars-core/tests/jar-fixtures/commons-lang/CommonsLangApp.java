package app;

import org.apache.commons.lang3.StringUtils;

public final class CommonsLangApp {
    public static void run() {
        System.out.println(StringUtils.isEmpty(null));
        System.out.println(StringUtils.isEmpty(""));
        System.out.println(StringUtils.isEmpty(" "));
        System.out.println(StringUtils.isBlank(null));
        System.out.println(StringUtils.isBlank(""));
        System.out.println(StringUtils.isBlank(" \t"));
        System.out.println(StringUtils.isBlank("\u00a0"));
        System.out.println(StringUtils.isBlank(" jars "));
    }
}
