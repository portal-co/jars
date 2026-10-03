package org.apache.kafka.clients.producer;

public final class ProducerRecord<K, V> {
    public ProducerRecord(String topic, Integer partition, Long timestamp, K key, V value) {
    }
}
