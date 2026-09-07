package app;

import org.apache.commons.lang3.StringUtils;

public final class StringUtilsPredicates {
    public static void run() {
        System.out.println(StringUtils.isAlpha(null));
        System.out.println(StringUtils.isAlpha("jars"));
        System.out.println(StringUtils.isAlpha("jars1"));
        System.out.println(StringUtils.isAlpha(""));
        System.out.println(StringUtils.isAlpha("оксфорд"));
        System.out.println(StringUtils.isAlphanumeric("jars1"));
        System.out.println(StringUtils.isAlphanumeric(""));
        System.out.println(StringUtils.isNumeric(null));
        System.out.println(StringUtils.isNumeric("123"));
        System.out.println(StringUtils.isNumeric("12.3"));
        System.out.println(StringUtils.isNumeric(""));
        System.out.println(StringUtils.isAllLowerCase("jars"));
        System.out.println(StringUtils.isAllLowerCase("Jars"));
        System.out.println(StringUtils.isAllUpperCase("JARS"));
        System.out.println(StringUtils.isWhitespace(" \t\n"));
        System.out.println(StringUtils.isWhitespace(" j "));
        System.out.println(StringUtils.isWhitespace(null));
        System.out.println(StringUtils.isNumericSpace("12 3"));
        System.out.println(StringUtils.isAlphaSpace("ja rs"));
        System.out.println(StringUtils.isBlank(null));
        System.out.println(StringUtils.isBlank(""));
        System.out.println(StringUtils.isBlank(" \t"));
        System.out.println(StringUtils.isBlank("\u00a0"));
        System.out.println(StringUtils.isBlank("\u1680"));
        System.out.println(StringUtils.isBlank(" jars "));
        System.out.println(StringUtils.isBlank(new StringBuilder(" \t")));
        System.out.println(StringUtils.isBlank(new StringBuilder("jars")));
        System.out.println(StringUtils.isBlank(new StringBuilder("\u1680")));
    }


    public static void main(String[] args) {
        run();
    }
}
