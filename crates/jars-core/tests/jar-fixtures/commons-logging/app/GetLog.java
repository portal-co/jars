package app;

import org.apache.commons.logging.Log;
import org.apache.commons.logging.LogFactory;

public class GetLog {
    public static void run() {
        Log logger = LogFactory.getLog("app.GetLog");
        if (logger == null) {
            System.out.println("null");
        } else {
            System.out.println("logger");
        }
    }
}
