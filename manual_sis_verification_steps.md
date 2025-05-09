# Manual Verification Steps for Single-Index Snapshots

This guide provides step-by-step instructions to manually test the single-index snapshot creation and import functionality using `curl` commands.

## Prerequisites

1.  **Meilisearch Binary**: Ensure you have a compiled Meilisearch binary that includes the single-index snapshot feature.
2.  **curl**: Ensure `curl` is installed on your system.
3.  **jq (optional but recommended)**: `jq` is a command-line JSON processor that helps in pretty-printing JSON responses. If you don't have it, responses will be harder to read. Install it via your system's package manager (e.g., `brew install jq` on macOS, `sudo apt-get install jq` on Debian/Ubuntu).

## Setup

### 1. Create a Snapshot Directory

First, create a directory where Meilisearch will store the snapshots.

```bash
mkdir ./ms_snapshots
```

### 2. Start Meilisearch

Open a terminal window (Terminal 1) and start your Meilisearch instance, pointing it to the snapshot directory you just created. Replace `./your_meilisearch_binary` with the actual path to your binary.

```bash
./your_meilisearch_binary --snapshot-dir ./ms_snapshots --db-path ./ms_data
```

Keep this terminal window open. Meilisearch will log its activity here. By default, it will run on `http://localhost:7700`. We are not setting a master key for simplicity in these manual steps.

## Testing Steps

Open a new terminal window (Terminal 2) for running the `curl` commands.

### 1. Create an Index

Let's create an index named `movies_source`.

```bash
curl -X POST 'http://localhost:7700/indexes' \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "uid": "movies_source",
    "primaryKey": "id"
  }' | jq
```

You should see a task enqueued. Note the `taskUid`.

### 2. Check Task Status (Optional)

You can check the status of the index creation task. Replace `TASK_UID_HERE` with the `taskUid` from the previous step.

```bash
curl 'http://localhost:7700/tasks/TASK_UID_HERE' | jq
```
Wait until the status is `succeeded`.

### 3. Add Documents to the Index

Let's add some documents to `movies_source`.

```bash
curl -X POST 'http://localhost:7700/indexes/movies_source/documents' \
  -H 'Content-Type: application/json' \
  --data-binary '[
    { "id": 1, "title": "Mad Max: Fury Road", "genre": "Action" },
    { "id": 2, "title": "Interstellar", "genre": "Sci-Fi" },
    { "id": 3, "title": "The Lord of the Rings", "genre": "Fantasy" }
  ]' | jq
```
Note the `taskUid`. Wait for this task to succeed (check status as above if needed).

### 4. Search the Index (Optional)

Verify the documents are indexed.

```bash
curl -X POST 'http://localhost:7700/indexes/movies_source/search' \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "q": "Mad Max"
  }' | jq
```
You should see "Mad Max: Fury Road" in the results.

### 5. Create a Snapshot for the Index

Now, let's create a snapshot for the `movies_source` index.

```bash
curl -X POST "http://localhost:7700/indexes/movies_source/snapshots" \
  -H 'Content-Type: application/json' \
  --data-binary '{}' | jq
```
Note the `taskUid`. This task will create a snapshot file in the `./ms_snapshots` directory.

### 6. Check Snapshot Creation Task Status

Wait for the snapshot creation task to complete. Replace `TASK_UID_HERE` with the `taskUid` from the snapshot creation step.

```bash
curl 'http://localhost:7700/tasks/TASK_UID_HERE' | jq
```
Once the status is `succeeded`, look at the `details` field. It should contain a `dumpUid` (this is the unique snapshot identifier, e.g., `YYYYMMDD-HHMMSS` or a similar unique string).
The `dumpUid` in the task details is actually just the unique part of the filename (the `SNAPSHOT_UID`).
The full filename will be constructed as `INDEX_UID-SNAPSHOT_UID.snapshot.tar.gz`. For our example, it would be `movies_source-YOUR_SNAPSHOT_UID.snapshot.tar.gz` where `YOUR_SNAPSHOT_UID` is the value from `dumpUid`.

Verify this file exists in your `./ms_snapshots` directory. For example, if the `index_uid` is `movies_source` and the `dumpUid` (snapshot UID) from the task details is `20250508-123456`, the file will be `movies_source-20250508-123456.snapshot.tar.gz`.

Let's assume the snapshot filename generated is `movies_source-YOUR_SNAPSHOT_UID.snapshot.tar.gz`. **You will need this exact filename for the next step.**

### 7. Import the Snapshot into a New Index

Let's import the snapshot into a new index named `movies_target`.
**Replace `movies_source-YOUR_SNAPSHOT_UID.snapshot.tar.gz` with the actual filename of your snapshot.**

```bash
curl -X POST 'http://localhost:7700/snapshots/import' \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "sourceSnapshotFilename": "movies_source-YOUR_SNAPSHOT_UID.snapshot.tar.gz",
    "targetIndexUid": "movies_target"
  }' | jq
```
Note the `taskUid`.

### 8. Check Snapshot Import Task Status

Wait for the import task to complete. Replace `TASK_UID_HERE` with the `taskUid` from the import step.

```bash
curl 'http://localhost:7700/tasks/TASK_UID_HERE' | jq
```
Wait until the status is `succeeded`.

### 9. Verify the New Index

Check if the `movies_target` index exists and has the documents.

Get index info:
```bash
curl 'http://localhost:7700/indexes/movies_target' | jq
```

Search the new index:
```bash
curl -X POST 'http://localhost:7700/indexes/movies_target/search' \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "q": "Interstellar"
  }' | jq
```
You should see "Interstellar" in the results.

You have now successfully created a snapshot of a single index and imported it as a new index with a different name!

## Cleanup (Optional)

1.  Stop Meilisearch by pressing `Ctrl+C` in Terminal 1.
2.  You can remove the data and snapshot directories:
    ```bash
    rm -rf ./ms_data
    rm -rf ./ms_snapshots
    ```
```
