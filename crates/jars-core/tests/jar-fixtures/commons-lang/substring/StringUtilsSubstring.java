package app;

import org.apache.commons.lang3.StringUtils;

public final class StringUtilsSubstring {
    public static void run() {
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
    }


    public static void main(String[] args) {
        run();
    }
}
