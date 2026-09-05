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
        System.out.println(StringUtils.isNotEmpty(null));
        System.out.println(StringUtils.isNotEmpty("jars"));
        System.out.println(StringUtils.trim(null));
        System.out.println(StringUtils.trim("  jars  "));
        System.out.println(StringUtils.trim("\u00a0 x \u00a0"));
        System.out.println(StringUtils.trimToNull(null));
        System.out.println(StringUtils.trimToNull("  "));
        System.out.println(StringUtils.trimToNull(" jars "));
        System.out.println(StringUtils.trimToEmpty(null));
        System.out.println(StringUtils.trimToEmpty("  "));
        System.out.println(StringUtils.upperCase(null));
        System.out.println(StringUtils.upperCase("jars"));
        System.out.println(StringUtils.lowerCase("JaRs"));
        System.out.println(StringUtils.capitalize(null));
        System.out.println(StringUtils.capitalize("jars"));
        System.out.println(StringUtils.capitalize("Jars"));
        System.out.println(StringUtils.capitalize("\u01c6ars"));
        System.out.println(StringUtils.uncapitalize("Jars"));
        System.out.println(StringUtils.uncapitalize("jars"));
        System.out.println(StringUtils.uncapitalize("\u01c5ars"));
        System.out.println(StringUtils.reverse(null));
        System.out.println(StringUtils.reverse("jars"));
        System.out.println(StringUtils.reverse("\ud83d\ude00a"));
        System.out.println(StringUtils.defaultString(null, "def"));
        System.out.println(StringUtils.defaultString("jars", "def"));
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
