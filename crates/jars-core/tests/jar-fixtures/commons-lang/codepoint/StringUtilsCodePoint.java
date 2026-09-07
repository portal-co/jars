package app;

import org.apache.commons.lang3.StringUtils;

public final class StringUtilsCodePoint {
    public static void run() {
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
    }


    public static void main(String[] args) {
        run();
    }
}
