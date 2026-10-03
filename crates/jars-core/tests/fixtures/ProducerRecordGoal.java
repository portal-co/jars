import org.apache.kafka.clients.producer.ProducerRecord;

public class ProducerRecordGoal {
    public static void main(String[] args) {
        new ProducerRecord<String, String>("events", null, null, "key", "value");
        System.out.println("record created");
    }
}
