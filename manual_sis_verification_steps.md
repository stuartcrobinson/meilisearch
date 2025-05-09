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

Open a terminal window (Terminal 1) and start your Meilisearch instance, pointing it to the snapshot directory you just created.
If you built Meilisearch in debug mode (e.g., using `cargo build`), the binary is typically located at `target/debug/meilisearch`.
If you built in release mode (e.g., `cargo build --release`), it's at `target/release/meilisearch`.

The following command assumes you have a debug build. If you used a release build (`cargo build --release`), change `debug` to `release` in the path.

```bash
./target/debug/meilisearch --snapshot-dir ./ms_snapshots --db-path ./ms_data
```

Alternatively, you can use `cargo run` which will compile and run the binary. If you use `cargo run`, you need to pass arguments after `--`:
```bash
# Example using cargo run (from the root of the meilisearch project)
# cargo run -- --snapshot-dir ./ms_snapshots --db-path ./ms_data
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


## List all the indexes
```
curl -X GET 'http://localhost:7700/indexes' | jq
```

## Cleanup (Optional)

1.  Stop Meilisearch by pressing `Ctrl+C` in Terminal 1.
2.  You can remove the data and snapshot directories:
    ```bash
    rm -rf ./ms_data
    rm -rf ./ms_snapshots
    ```

# this works perfectly. amazing

```
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/indexes' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{
                                  "uid": "movies_source",
                                  "primaryKey": "id"
                                }' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   182  100   126  100    56   4181   1858 --:--:-- --:--:-- --:--:--  7583
{
  "taskUid": 0,
  "indexUid": "movies_source",
  "status": "enqueued",
  "type": "indexCreation",
  "enqueuedAt": "2025-05-09T02:58:44.473557Z"
}
stuart@Stuarts-MacBook-Pro ~> curl 'http://localhost:7700/tasks/0' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   306  100   306    0     0  38064      0 --:--:-- --:--:-- --:--:--  149k
{
  "uid": 0,
  "batchUid": 0,
  "indexUid": "movies_source",
  "status": "succeeded",
  "type": "indexCreation",
  "canceledBy": null,
  "details": {
    "primaryKey": "id"
  },
  "error": null,
  "duration": "PT0.104722S",
  "enqueuedAt": "2025-05-09T02:58:44.473557Z",
  "startedAt": "2025-05-09T02:58:44.49509Z",
  "finishedAt": "2025-05-09T02:58:44.599812Z"
}
stuart@Stuarts-MacBook-Pro ~>
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/indexes/movies_source/documents' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '[
                                  { "id": 1, "title": "Mad Max: Fury Road", "genre": "Action" },
                                  { "id": 2, "title": "Interstellar", "genre": "Sci-Fi" },
                                  { "id": 3, "title": "The Lord of the Rings", "genre": "Fantasy" }
                                ]' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   340  100   137  100   203   4569   6771 --:--:-- --:--:-- --:--:-- 14166
{
  "taskUid": 1,
  "indexUid": "movies_source",
  "status": "enqueued",
  "type": "documentAdditionOrUpdate",
  "enqueuedAt": "2025-05-09T02:59:45.409953Z"
}
stuart@Stuarts-MacBook-Pro ~> curl 'http://localhost:7700/tasks/1' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   343  100   343    0     0  42576      0 --:--:-- --:--:-- --:--:--  167k
{
  "uid": 1,
  "batchUid": 1,
  "indexUid": "movies_source",
  "status": "succeeded",
  "type": "documentAdditionOrUpdate",
  "canceledBy": null,
  "details": {
    "receivedDocuments": 3,
    "indexedDocuments": 3
  },
  "error": null,
  "duration": "PT0.785620S",
  "enqueuedAt": "2025-05-09T02:59:45.409953Z",
  "startedAt": "2025-05-09T02:59:45.430745Z",
  "finishedAt": "2025-05-09T02:59:46.216365Z"
}
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/indexes/movies_source/search' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{
                                  "q": "Mad Max"
                                }' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   173  100   149  100    24  15464   2490 --:--:-- --:--:-- --:--:-- 57666
{
  "hits": [
    {
      "id": 1,
      "title": "Mad Max: Fury Road",
      "genre": "Action"
    }
  ],
  "query": "Mad Max",
  "processingTimeMs": 1,
  "limit": 20,
  "offset": 0,
  "estimatedTotalHits": 1
}
stuart@Stuarts-MacBook-Pro ~>
stuart@Stuarts-MacBook-Pro ~>
stuart@Stuarts-MacBook-Pro ~> curl -X POST "http://localhost:7700/indexes/movies_source/snapshots" \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{}' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   142  100   140  100     2   5009     71 --:--:-- --:--:-- --:--:--  6454
{
  "taskUid": 2,
  "indexUid": "movies_source",
  "status": "enqueued",
  "type": "singleIndexSnapshotCreation",
  "enqueuedAt": "2025-05-09T03:00:33.658405Z"
}
stuart@Stuarts-MacBook-Pro ~> curl 'http://localhost:7700/tasks/2' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   351  100   351    0     0  48280      0 --:--:-- --:--:-- --:--:--  342k
{
  "uid": 2,
  "batchUid": 2,
  "indexUid": "movies_source",
  "status": "succeeded",
  "type": "singleIndexSnapshotCreation",
  "canceledBy": null,
  "details": {
    "dumpUid": "fe081318-9c75-4b2f-b140-c975404c4613"
  },
  "error": null,
  "duration": "PT0.050054S",
  "enqueuedAt": "2025-05-09T03:00:33.658405Z",
  "startedAt": "2025-05-09T03:00:33.67837Z",
  "finishedAt": "2025-05-09T03:00:33.728424Z"
}
stuart@Stuarts-MacBook-Pro ~> curl -X POST "http://localhost:7700/indexes/movies_source/snapshots" \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{}' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   142  100   140  100     2   4583     65 --:--:-- --:--:-- --:--:--  5680
{
  "taskUid": 3,
  "indexUid": "movies_source",
  "status": "enqueued",
  "type": "singleIndexSnapshotCreation",
  "enqueuedAt": "2025-05-09T03:01:09.912614Z"
}
stuart@Stuarts-MacBook-Pro ~> curl 'http://localhost:7700/tasks/3' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   351  100   351    0     0  47032      0 --:--:-- --:--:-- --:--:--  342k
{
  "uid": 3,
  "batchUid": 3,
  "indexUid": "movies_source",
  "status": "succeeded",
  "type": "singleIndexSnapshotCreation",
  "canceledBy": null,
  "details": {
    "dumpUid": "9dbafa26-bb2d-42c6-9565-52a7b5ef65ad"
  },
  "error": null,
  "duration": "PT0.048758S",
  "enqueuedAt": "2025-05-09T03:01:09.912614Z",
  "startedAt": "2025-05-09T03:01:09.93542Z",
  "finishedAt": "2025-05-09T03:01:09.984178Z"
}
stuart@Stuarts-MacBook-Pro ~>
stuart@Stuarts-MacBook-Pro ~>
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/snapshots/import' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{
                                  "sourceSnapshotFilename": "movies_source-9dbafa26-bb2d-42c6-9565-52a7b5ef65ad.snapshot.tar.gz",
                                  "targetIndexUid": "movies_target!"
                                }' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   418  100   274  100   144  36635  19253 --:--:-- --:--:-- --:--:--  408k
{
  "message": "Invalid `targetIndexUid` provided: 'movies_target!'. Index UID can only be composed of alphanumeric characters, hyphens (-), and underscores (_).",
  "code": "invalid_index_uid",
  "type": "invalid_request",
  "link": "https://docs.meilisearch.com/errors#invalid_index_uid"
}
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/snapshots/import' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{
                                  "sourceSnapshotFilename": "movies_source-9dbafa26-bb2d-42c6-9565-52a7b5ef65ad.snapshot.tar.gz",
                                  "targetIndexUid": "movies_targetttt"
                                }' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   287  100   141  100   146   5107   5288 --:--:-- --:--:-- --:--:-- 13045
{
  "taskUid": 4,
  "indexUid": "movies_targetttt",
  "status": "enqueued",
  "type": "singleIndexSnapshotImport",
  "enqueuedAt": "2025-05-09T03:03:26.528171Z"
}
stuart@Stuarts-MacBook-Pro ~> curl 'http://localhost:7700/tasks/4' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   407  100   407    0     0  49381      0 --:--:-- --:--:-- --:--:--  198k
{
  "uid": 4,
  "batchUid": 4,
  "indexUid": "movies_targetttt",
  "status": "succeeded",
  "type": "singleIndexSnapshotImport",
  "canceledBy": null,
  "details": {
    "originalFilter": "Importing snapshot 9dbafa26-bb2d-42c6-9565-52a7b5ef65ad into index movies_targetttt"
  },
  "error": null,
  "duration": "PT0.050276S",
  "enqueuedAt": "2025-05-09T03:03:26.528171Z",
  "startedAt": "2025-05-09T03:03:26.548948Z",
  "finishedAt": "2025-05-09T03:03:26.599224Z"
}
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/snapshots/import' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{
                                  "sourceSnapshotFilename": "movies_source-9dbafa26-bb2d-42c6-9565-52a7b5ef65ad.snapshot.tar.gz",
                                  "targetIndexUid": "movies_target"
                                }' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   281  100   138  100   143   4662   4830 --:--:-- --:--:-- --:--:-- 12217
{
  "taskUid": 5,
  "indexUid": "movies_target",
  "status": "enqueued",
  "type": "singleIndexSnapshotImport",
  "enqueuedAt": "2025-05-09T03:04:23.190839Z"
}
stuart@Stuarts-MacBook-Pro ~> curl 'http://localhost:7700/tasks/5' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   401  100   401    0     0  54505      0 --:--:-- --:--:-- --:--:--  391k
{
  "uid": 5,
  "batchUid": 5,
  "indexUid": "movies_target",
  "status": "succeeded",
  "type": "singleIndexSnapshotImport",
  "canceledBy": null,
  "details": {
    "originalFilter": "Importing snapshot 9dbafa26-bb2d-42c6-9565-52a7b5ef65ad into index movies_target"
  },
  "error": null,
  "duration": "PT0.050044S",
  "enqueuedAt": "2025-05-09T03:04:23.190839Z",
  "startedAt": "2025-05-09T03:04:23.212312Z",
  "finishedAt": "2025-05-09T03:04:23.262356Z"
}
stuart@Stuarts-MacBook-Pro ~> curl 'http://localhost:7700/indexes/movies_target' | jq

  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   125  100   125    0     0  17556      0 --:--:-- --:--:-- --:--:--  122k
{
  "uid": "movies_target",
  "createdAt": "2025-05-09T02:58:44.496479Z",
  "updatedAt": "2025-05-09T03:04:23.239338Z",
  "primaryKey": "id"
}
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/indexes/movies_target/search' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{
                                  "q": "Interstellar"
                                }' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   177  100   148  100    29   9108   1784 --:--:-- --:--:-- --:--:-- 17700
{
  "hits": [
    {
      "id": 2,
      "title": "Interstellar",
      "genre": "Sci-Fi"
    }
  ],
  "query": "Interstellar",
  "processingTimeMs": 8,
  "limit": 20,
  "offset": 0,
  "estimatedTotalHits": 1
}
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/indexes/movies_target/search' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{
                                  "q": "Inte"
                                }' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   161  100   140  100    21  15141   2271 --:--:-- --:--:-- --:--:-- 53666
{
  "hits": [
    {
      "id": 2,
      "title": "Interstellar",
      "genre": "Sci-Fi"
    }
  ],
  "query": "Inte",
  "processingTimeMs": 0,
  "limit": 20,
  "offset": 0,
  "estimatedTotalHits": 1
}
stuart@Stuarts-MacBook-Pro ~> curl -X POST 'http://localhost:7700/indexes/movies_target/search' \
                                    -H 'Content-Type: application/json' \
                                    --data-binary '{
                                  "q": "Intre"
                                }' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   163  100   141  100    22  15670   2444 --:--:-- --:--:-- --:--:-- 54333
{
  "hits": [
    {
      "id": 2,
      "title": "Interstellar",
      "genre": "Sci-Fi"
    }
  ],
  "query": "Intre",
  "processingTimeMs": 1,
  "limit": 20,
  "offset": 0,
  "estimatedTotalHits": 1
}
stuart@Stuarts-MacBook-Pro ~> curl -X GET 'http://localhost:7700/indexes'
{"results":[{"uid":"movies_source","createdAt":"2025-05-09T02:58:44.496479Z","updatedAt":"2025-05-09T02:59:46.083495Z","primaryKey":"id"},{"uid":"movies_target","createdAt":"2025-05-09T02:58:44.496479Z","updatedAt":"2025-05-09T02:59:46.083495Z","primaryKey":"id"},{"uid":"movies_targetttt","createdAt":"2025-05-09T02:58:44.496479Z","updatedAt":"2025-05-09T02:59:46.083495Z","primaryKey":"id"}],"offset":0,"limit":20,"total":3}⏎
stuart@Stuarts-MacBook-Pro ~> curl -X GET 'http://localhost:7700/indexes' | jq
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   426  100   426    0     0  55708      0 --:--:-- --:--:-- --:--:--  416k
{
  "results": [
    {
      "uid": "movies_source",
      "createdAt": "2025-05-09T02:58:44.496479Z",
      "updatedAt": "2025-05-09T02:59:46.083495Z",
      "primaryKey": "id"
    },
    {
      "uid": "movies_target",
      "createdAt": "2025-05-09T02:58:44.496479Z",
      "updatedAt": "2025-05-09T02:59:46.083495Z",
      "primaryKey": "id"
    },
    {
      "uid": "movies_targetttt",
      "createdAt": "2025-05-09T02:58:44.496479Z",
      "updatedAt": "2025-05-09T02:59:46.083495Z",
      "primaryKey": "id"
    }
  ],
  "offset": 0,
  "limit": 20,
  "total": 3
}
stuart@Stuarts-MacBook-Pro ~>
```