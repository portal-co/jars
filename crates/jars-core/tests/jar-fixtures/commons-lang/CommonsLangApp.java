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
        System.out.println(StringUtils.isBlank("\u1680"));
        System.out.println(StringUtils.isBlank(" jars "));
        System.out.println(StringUtils.isBlank(new StringBuilder(" \t")));
        System.out.println(StringUtils.isBlank(new StringBuilder("jars")));
        System.out.println(StringUtils.isBlank(new StringBuilder("\u1680")));
        System.out.println(new StringBuilder("😀").length());
        System.out.println(builderLength(1));
    }

    private static int builderLength(int takeLength) {
        StringBuilder builder = new StringBuilder("😀");
        if (takeLength != 0) {
            return builder.length();
        }
        return -1;
    }
}
