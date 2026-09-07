package app;

import org.apache.commons.lang3.StringUtils;

public final class StringUtilsTrim {
    public static void run() {
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
    }


    public static void main(String[] args) {
        run();
    }
}
