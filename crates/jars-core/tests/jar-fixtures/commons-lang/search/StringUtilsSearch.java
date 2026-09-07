package app;

import org.apache.commons.lang3.StringUtils;

public final class StringUtilsSearch {
    public static void run() {
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


    public static void main(String[] args) {
        run();
    }
}
