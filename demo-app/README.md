# New Relic Python Demo App

A simple Python demonstration application that sends both APM (Application Performance Monitoring) metrics and custom metrics to New Relic.

## Features

- **APM Instrumentation**: Automatic tracking of background tasks and transactions
- **Custom Metrics**: Counters, gauges, and performance metrics
- **Custom Events**: Business events with custom attributes
- **Error Tracking**: Automatic error reporting and exception handling
- **Transaction Attributes**: Custom attributes added to transactions

## Prerequisites

- Python 3.7 or higher
- [uv](https://docs.astral.sh/uv/) - Fast Python package installer (install with `curl -LsSf https://astral.sh/uv/install.sh | sh`)
- A New Relic account (sign up at https://newrelic.com/signup)
- New Relic License Key

## Setup

### 1. Install Dependencies

```bash
uv pip install -r requirements.txt
```

Or use uv to run directly without installing:

```bash
uv run demo_app.py
```

### 2. Configure New Relic

Edit the `newrelic.ini` file and replace `YOUR_LICENSE_KEY_HERE` with your actual New Relic license key:

```ini
license_key = YOUR_LICENSE_KEY_HERE
```

You can find your license key in your New Relic account:
- Go to: https://one.newrelic.com
- Click on your account name (bottom left)
- Select "API keys"
- Copy your "License key" (or "INGEST - LICENSE")

### 3. Run the Demo

```bash
uv run demo_app.py
```

Or if you installed dependencies manually:

```bash
python demo_app.py
```

The application will run for 10 iterations (about 2 minutes), sending various metrics to New Relic. You can stop it early with `Ctrl+C`.

## What Gets Sent to New Relic

### APM Data

- **Transactions**: Each order processing and batch operation is tracked as a transaction
- **Response Times**: Automatic timing of all operations
- **Throughput**: Request rates and transaction counts
- **Errors**: Automatic error tracking with stack traces (10% simulated error rate)

### Custom Metrics

- `Custom/Orders/Processed` - Counter of processed orders
- `Custom/Orders/TotalAmount` - Total order amounts
- `Custom/Batch/ItemsProcessed` - Number of items in each batch
- `Custom/Batch/Duration` - Time taken to process batches
- `Custom/Batch/ItemsPerSecond` - Processing rate
- `Custom/System/CPU` - Simulated CPU usage gauge
- `Custom/System/Memory` - Simulated memory usage gauge
- `Custom/Queue/Depth` - Simulated queue depth

### Custom Events

- `BatchProcessed` - Fired after each batch with size and duration
- `IterationCompleted` - Fired after each iteration with counters

### Transaction Attributes

- `order_id` - Unique identifier for each order
- `order_amount` - Dollar amount of each order

## Viewing Data in New Relic

### APM & Services

1. Go to: https://one.newrelic.com
2. Click on "APM & Services" in the left menu
3. Look for "Python Demo App"
4. You'll see transaction traces, error rates, and throughput

### Custom Metrics

1. In New Relic, go to "Query your data" or use the query builder
2. Use NRQL queries like:

```sql
-- View custom order metrics
SELECT sum(Custom/Orders/Processed) as 'Orders',
       sum(Custom/Orders/TotalAmount) as 'Revenue'
FROM Metric
SINCE 30 minutes ago

-- View batch processing performance
SELECT average(Custom/Batch/ItemsPerSecond) as 'Avg Items/Sec',
       max(Custom/Batch/Duration) as 'Max Duration'
FROM Metric
SINCE 30 minutes ago

-- View system gauges
SELECT latest(Custom/System/CPU) as 'CPU',
       latest(Custom/System/Memory) as 'Memory',
       latest(Custom/Queue/Depth) as 'Queue Depth'
FROM Metric
SINCE 30 minutes ago TIMESERIES
```

### Custom Events

```sql
-- View batch processing events
SELECT * FROM BatchProcessed SINCE 30 minutes ago

-- View iteration completions
SELECT * FROM IterationCompleted SINCE 30 minutes ago
```

## Customizing the Demo

You can modify `demo_app.py` to:

- Change the number of iterations (default: 10)
- Adjust the delay between iterations (default: 10 seconds)
- Add more custom metrics or events
- Modify the simulated error rate
- Add your own business logic

## Troubleshooting

### No data appearing in New Relic?

1. Verify your license key is correct in `newrelic.ini`
2. Check the agent log at `/tmp/newrelic-python-agent.log`
3. Ensure you have network connectivity to New Relic's collectors
4. Wait 2-3 minutes for data to appear (there's a small delay)

### Log Level

To see more detailed logging, change the log level in `newrelic.ini`:

```ini
log_level = debug
```

## Additional Resources

- [New Relic Python Agent Documentation](https://docs.newrelic.com/docs/apm/agents/python-agent/getting-started/introduction-new-relic-python/)
- [Custom Metrics API](https://docs.newrelic.com/docs/apm/agents/python-agent/python-agent-api/recordcustommetric-python-agent-api/)
- [Custom Events API](https://docs.newrelic.com/docs/apm/agents/python-agent/python-agent-api/recordcustomevent-python-agent-api/)
- [NRQL Query Language](https://docs.newrelic.com/docs/query-your-data/nrql-new-relic-query-language/get-started/introduction-nrql-new-relics-query-language/)
