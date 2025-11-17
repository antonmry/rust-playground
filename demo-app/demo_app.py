#!/usr/bin/env python3
"""
New Relic Python Demo App
Demonstrates both APM instrumentation and custom metric reporting
"""
# /// script
# dependencies = [
#   "newrelic==10.2.0",
#   "requests==2.31.0",
# ]
# ///

import time
import random
import newrelic.agent
from newrelic.agent import (
    record_custom_metric,
    record_custom_event,
    add_custom_attribute,
    BackgroundTask
)

# Initialize the New Relic agent
newrelic.agent.initialize('newrelic.ini')


class DemoApplication:
    """Simple demo application that performs various operations"""

    def __init__(self):
        self.counter = 0

    @newrelic.agent.background_task()
    def process_order(self, order_id, amount):
        """Simulates processing an order - tracked as a transaction"""
        print(f"Processing order {order_id} for ${amount}")

        # Add custom attributes to the current transaction
        add_custom_attribute('order_id', order_id)
        add_custom_attribute('order_amount', amount)

        # Simulate some work
        time.sleep(random.uniform(0.1, 0.5))

        # Record custom metrics
        record_custom_metric('Custom/Orders/Processed', 1)
        record_custom_metric('Custom/Orders/TotalAmount', amount)

        # Simulate occasional errors
        if random.random() < 0.1:
            print(f"  Error processing order {order_id}")
            newrelic.agent.notice_error()
            raise Exception(f"Failed to process order {order_id}")

        print(f"  Order {order_id} completed successfully")
        return True

    @newrelic.agent.background_task()
    def process_batch(self, batch_size):
        """Processes a batch of items - demonstrates nested transactions"""
        print(f"\nStarting batch processing ({batch_size} items)")

        start_time = time.time()

        for i in range(batch_size):
            # Simulate item processing
            time.sleep(random.uniform(0.05, 0.15))
            self.counter += 1

        duration = time.time() - start_time

        # Record custom metrics
        record_custom_metric('Custom/Batch/ItemsProcessed', batch_size)
        record_custom_metric('Custom/Batch/Duration', duration)
        record_custom_metric('Custom/Batch/ItemsPerSecond', batch_size / duration)

        # Record custom event
        record_custom_event('BatchProcessed', {
            'batch_size': batch_size,
            'duration': duration,
            'items_per_second': batch_size / duration
        })

        print(f"Batch completed: {batch_size} items in {duration:.2f}s ({batch_size/duration:.2f} items/sec)")

    def send_gauge_metrics(self):
        """Sends gauge-type metrics (current values)"""
        # Simulate various system metrics
        cpu_usage = random.uniform(10, 90)
        memory_usage = random.uniform(30, 80)
        queue_depth = random.randint(0, 100)

        record_custom_metric('Custom/System/CPU', cpu_usage)
        record_custom_metric('Custom/System/Memory', memory_usage)
        record_custom_metric('Custom/Queue/Depth', queue_depth)

        print(f"Gauge metrics - CPU: {cpu_usage:.1f}%, Memory: {memory_usage:.1f}%, Queue: {queue_depth}")


def main():
    """Main execution loop"""
    print("=== New Relic Python Demo App ===")
    print("Starting to send metrics to New Relic...\n")

    app = DemoApplication()
    iteration = 0

    try:
        while iteration < 10:  # Run 10 iterations for demo
            iteration += 1
            print(f"\n--- Iteration {iteration} ---")

            # Process some orders
            for order_num in range(random.randint(2, 5)):
                order_id = f"ORD-{iteration:03d}-{order_num:02d}"
                amount = round(random.uniform(10, 500), 2)

                try:
                    app.process_order(order_id, amount)
                except Exception as e:
                    # Error is already reported to New Relic
                    pass

            # Process a batch
            batch_size = random.randint(5, 20)
            app.process_batch(batch_size)

            # Send gauge metrics
            app.send_gauge_metrics()

            # Record a custom event for the iteration
            record_custom_event('IterationCompleted', {
                'iteration': iteration,
                'total_processed': app.counter
            })

            print(f"\nTotal items processed so far: {app.counter}")
            print("Waiting 10 seconds before next iteration...")
            time.sleep(10)

        print("\n=== Demo completed successfully ===")
        print("Check your New Relic dashboard for metrics and traces!")

    except KeyboardInterrupt:
        print("\n\n=== Demo stopped by user ===")
        print("Check your New Relic dashboard for metrics and traces!")

    # Ensure all data is sent before exiting
    newrelic.agent.shutdown_agent(timeout=10)


if __name__ == '__main__':
    main()
