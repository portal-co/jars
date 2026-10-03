import org.apache.commons.logging.Log;
import org.apache.commons.logging.LogFactory;

public final class LogFactorySmoke {
    public static void main(String[] args) {
        Log log = LogFactory.getLog("smoke");
        System.out.println(log != null);
    }
}
