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
        System.out.println(StringUtils.substring(null, 1, 2));
        System.out.println(StringUtils.substring("jars", 0, 4));
        System.out.println(StringUtils.substring("jars", -3, 3));
        System.out.println(StringUtils.substring("jars", 2, 1));
        System.out.println(StringUtils.substringBefore("jar.jar.jar", "."));
        System.out.println(StringUtils.substringBefore("jarJar", "."));
        System.out.println(StringUtils.substringAfter("jar.jar.jar", "."));
        System.out.println(StringUtils.substringAfter("jarJar", "."));
        System.out.println(StringUtils.substringBeforeLast("jar.jar.jar", "."));
        System.out.println(StringUtils.substringAfterLast("jar.jar.jar", "."));
        System.out.println(StringUtils.chop("jars"));
        System.out.println(StringUtils.chop("j"));
        System.out.println(StringUtils.chop(""));
        System.out.println(StringUtils.chop(null));
        System.out.println(StringUtils.chomp("jars\n"));
        System.out.println(StringUtils.chomp("jars\r\n"));
        System.out.println(StringUtils.chomp("jars"));
        System.out.println(StringUtils.compare(null, "jars", false));
        System.out.println(StringUtils.compare("jars", null, false));
        System.out.println(StringUtils.compare("jars", "jars", false));
        System.out.println(StringUtils.compare("jars", "kars", false));
        System.out.println(StringUtils.countMatches("jarajar", 'a'));
        System.out.println(StringUtils.countMatches(null, 'a'));
        System.out.println(StringUtils.countMatches("jars", 'z'));
        System.out.println(StringUtils.wrap(null, "'"));
        System.out.println(StringUtils.wrap("ab", "'"));
        System.out.println(StringUtils.wrap("ab", ""));
        System.out.println(StringUtils.deleteWhitespace("   j  a r s   "));
        System.out.println(StringUtils.repeat('a', 3));
        System.out.println(StringUtils.repeat('a', 0));
        System.out.println(StringUtils.repeat('a', -1));
        System.out.println(StringUtils.join(new int[] {1, 2, 3}, '-', 0, 3));
        System.out.println(StringUtils.join(new int[] {}, '-', 0, 0));
    }

    private static int builderLength(int takeLength) {
        StringBuilder builder = new StringBuilder("😀");
        if (takeLength != 0) {
            return builder.length();
        }
        return -1;
    }
}
