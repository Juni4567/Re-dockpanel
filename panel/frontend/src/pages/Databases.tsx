import { useState, useEffect, useRef, useCallback, FormEvent } from "react";
import { api, ApiError } from "../api";
import { formatDate } from "../utils/format";
import { useAuth } from "../context/AuthContext";
import ConfirmDialog from "../components/ConfirmDialog";
import { showGlobalNotification } from "../context/NotificationContext";

interface Site {
  id: string;
  domain: string;
}

// A Docker Stack a database can be provisioned against instead of a Site
// (GH #64: standalone DB provisioning for Docker Apps). Stacks are admin-only
// throughout the panel, so this list is only ever fetched/shown for admins.
interface Stack {
  id: string;
  name: string;
}

interface Database {
  id: string;
  site_id: string | null;
  stack_id: string | null;
  name: string;
  engine: string;
  db_user: string;
  container_id: string | null;
  port: number | null;
  created_at: string;
}

interface Credentials {
  host: string;
  port: number;
  database: string;
  username: string;
  password: string;
  engine: string;
  connection_string: string;
  internal_host: string;
}

interface QueryResult {
  columns: string[];
  rows: string[][];
  row_count: number;
  execution_time_ms: number;
  truncated: boolean;
}

const engineLabels: Record<string, string> = {
  postgres: "PostgreSQL",
  mysql: "MySQL",
  mariadb: "MariaDB",
};

/* ─── Advanced Database Manager (phpMyAdmin Alternative) ───────────── */

type ManagerTab = "browse" | "structure" | "sql" | "import" | "export" | "find_replace";

interface DumpFile {
  filename: string;
  size_bytes: number;
  created_at: string;
  unsupported?: string;
  import_blocked?: string;
}

interface SchemaOverview {
  tables?: QueryResult;
  foreign_keys?: QueryResult;
}

function formatBytes(n: number): string {
  if (n >= 1_073_741_824) return `${(n / 1_073_741_824).toFixed(1)} GB`;
  if (n >= 1_048_576) return `${(n / 1_048_576).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${n} B`;
}

function AdvancedDatabaseManager({
  database,
  initialTab = "browse",
  onClose,
}: {
  database: Database;
  initialTab?: ManagerTab;
  onClose: () => void;
}) {
  const [activeTab, setActiveTab] = useState<ManagerTab>(initialTab);
  const [tables, setTables] = useState<string[]>([]);
  const [tableDetails, setTableDetails] = useState<Record<string, { rows: string; sizeKb: string }>>({});
  const [foreignKeys, setForeignKeys] = useState<string[][]>([]);
  const [fkColumns, setFkColumns] = useState<string[]>([]);
  const [tablesLoading, setTablesLoading] = useState(true);
  const [selectedTable, setSelectedTable] = useState<string | null>(null);

  // Common quote char for identifiers: Postgres uses double-quotes, MySQL/MariaDB backticks
  const q = database.engine === "postgres" ? '"' : "`";

  // Lethal action confirmations
  const [confirmModal, setConfirmModal] = useState<{
    title: string;
    label: string;
    consequences: string[];
    confirmLabel?: string;
    onConfirm: () => void;
  } | null>(null);

  // Load tables & schema overview
  const loadTables = useCallback(async () => {
    setTablesLoading(true);
    try {
      const overview = await api.get<SchemaOverview>(`/databases/${database.id}/schema-overview`);
      if (overview.tables?.rows) {
        const tNames: string[] = [];
        const details: Record<string, { rows: string; sizeKb: string }> = {};
        for (const r of overview.tables.rows) {
          tNames.push(r[0]);
          details[r[0]] = { rows: r[2] || "0", sizeKb: r[3] || "0" };
        }
        setTables(tNames);
        setTableDetails(details);
        if (tNames.length > 0 && !selectedTable) {
          setSelectedTable(tNames[0]);
        }
      } else {
        const res = await api.get<QueryResult>(`/databases/${database.id}/tables`);
        const tNames = res.rows.map((r) => r[0]);
        setTables(tNames);
        if (tNames.length > 0 && !selectedTable) {
          setSelectedTable(tNames[0]);
        }
      }
      if (overview.foreign_keys?.rows) {
        setForeignKeys(overview.foreign_keys.rows);
        setFkColumns(overview.foreign_keys.columns || []);
      }
    } catch {
      try {
        const res = await api.get<QueryResult>(`/databases/${database.id}/tables`);
        const tNames = res.rows.map((r) => r[0]);
        setTables(tNames);
        if (tNames.length > 0 && !selectedTable) setSelectedTable(tNames[0]);
      } catch {
        // Ignored
      }
    } finally {
      setTablesLoading(false);
    }
  }, [database.id, selectedTable]);

  useEffect(() => {
    loadTables();
  }, [loadTables]);

  /* ──────────────── Browse Data Tab State ──────────────── */
  const [browseRows, setBrowseRows] = useState<QueryResult | null>(null);
  const [browseLoading, setBrowseLoading] = useState(false);
  const [browseError, setBrowseError] = useState("");
  const [browsePage, setBrowsePage] = useState(0);
  const [pageSize, setPageSize] = useState(25);
  const [totalRows, setTotalRows] = useState<number | null>(null);
  const [sortCol, setSortCol] = useState<string | null>(null);
  const [sortDir, setSortDir] = useState<"ASC" | "DESC">("ASC");
  const [whereClause, setWhereClause] = useState("");
  const [showInsertModal, setShowInsertModal] = useState(false);
  const [insertColValues, setInsertColValues] = useState<Record<string, string>>({});
  const [inserting, setInserting] = useState(false);

  const loadBrowseData = useCallback(async (tableToLoad?: string) => {
    const tbl = tableToLoad || selectedTable;
    if (!tbl) return;
    setBrowseLoading(true);
    setBrowseError("");
    try {
      let query = `SELECT * FROM ${q}${tbl}${q}`;
      if (whereClause.trim()) {
        query += ` WHERE ${whereClause.trim()}`;
      }
      if (sortCol) {
        query += ` ORDER BY ${q}${sortCol}${q} ${sortDir}`;
      }
      query += ` LIMIT ${pageSize} OFFSET ${browsePage * pageSize}`;

      const res = await api.post<QueryResult>(`/databases/${database.id}/query`, { sql: query });
      setBrowseRows(res);

      try {
        const countSql = `SELECT COUNT(*) FROM ${q}${tbl}${q}${whereClause.trim() ? ` WHERE ${whereClause.trim()}` : ""}`;
        const countRes = await api.post<QueryResult>(`/databases/${database.id}/query`, { sql: countSql });
        if (countRes.rows?.[0]?.[0]) {
          setTotalRows(parseInt(countRes.rows[0][0], 10));
        }
      } catch {
        // Non-fatal
      }
    } catch (e) {
      setBrowseError(e instanceof Error ? e.message : "Failed to load table data");
    } finally {
      setBrowseLoading(false);
    }
  }, [database.id, selectedTable, q, whereClause, sortCol, sortDir, pageSize, browsePage]);

  useEffect(() => {
    if (activeTab === "browse" && selectedTable) {
      loadBrowseData();
    }
  }, [activeTab, selectedTable, browsePage, pageSize, sortCol, sortDir, loadBrowseData]);

  const handleSort = (col: string) => {
    if (sortCol === col) {
      setSortDir((d) => (d === "ASC" ? "DESC" : "ASC"));
    } else {
      setSortCol(col);
      setSortDir("ASC");
    }
    setBrowsePage(0);
  };

  const handleTruncateTable = (tbl: string) => {
    setConfirmModal({
      title: `Empty (Truncate) Table: ${tbl}`,
      label: `Are you sure you want to erase all data in table "${tbl}"?`,
      consequences: [
        `All rows and data inside table "${tbl}" will be immediately deleted`,
        "Auto-increment primary keys will be reset to 1",
        "This action cannot be undone unless you have a recent database backup",
      ],
      confirmLabel: "Empty Table (Truncate)",
      onConfirm: async () => {
        try {
          const sql = database.engine === "postgres" ? `TRUNCATE TABLE ${q}${tbl}${q} CASCADE` : `TRUNCATE TABLE ${q}${tbl}${q}`;
          await api.post(`/databases/${database.id}/query`, { sql });
          showGlobalNotification(`Table "${tbl}" emptied successfully`, "success");
          loadBrowseData();
          loadTables();
        } catch (e) {
          showGlobalNotification(e instanceof Error ? e.message : "Truncate failed", "error");
        } finally {
          setConfirmModal(null);
        }
      },
    });
  };

  const handleDropTable = (tbl: string) => {
    setConfirmModal({
      title: `Drop Table: ${tbl}`,
      label: `Are you sure you want to permanently DROP table "${tbl}"?`,
      consequences: [
        `The table definition and ALL data in "${tbl}" will be permanently destroyed`,
        "All associated column indexes, constraints, and triggers will be dropped",
        "Any application code or database foreign keys referencing this table will immediately fail",
      ],
      confirmLabel: "Permanently Drop Table",
      onConfirm: async () => {
        try {
          const sql = database.engine === "postgres" ? `DROP TABLE ${q}${tbl}${q} CASCADE` : `DROP TABLE ${q}${tbl}${q}`;
          await api.post(`/databases/${database.id}/query`, { sql });
          showGlobalNotification(`Table "${tbl}" dropped successfully`, "success");
          setSelectedTable(null);
          loadTables();
        } catch (e) {
          showGlobalNotification(e instanceof Error ? e.message : "Drop table failed", "error");
        } finally {
          setConfirmModal(null);
        }
      },
    });
  };

  const handleInsertRow = async () => {
    if (!selectedTable) return;
    const cols = Object.keys(insertColValues).filter((k) => insertColValues[k] !== "");
    if (cols.length === 0) {
      showGlobalNotification("Please provide at least one column value", "error");
      return;
    }
    setInserting(true);
    try {
      const colList = cols.map((c) => `${q}${c}${q}`).join(", ");
      const valList = cols.map((c) => {
        const v = insertColValues[c];
        return `'${v.replace(/'/g, "''")}'`;
      }).join(", ");
      const sql = `INSERT INTO ${q}${selectedTable}${q} (${colList}) VALUES (${valList})`;
      await api.post(`/databases/${database.id}/query`, { sql });
      showGlobalNotification("Row inserted successfully", "success");
      setShowInsertModal(false);
      setInsertColValues({});
      loadBrowseData();
      loadTables();
    } catch (e) {
      showGlobalNotification(e instanceof Error ? e.message : "Insert failed", "error");
    } finally {
      setInserting(false);
    }
  };

  /* ──────────────── Structure Tab State ──────────────── */
  const [structureCols, setStructureCols] = useState<QueryResult | null>(null);
  const [structureIndexes, setStructureIndexes] = useState<QueryResult | null>(null);
  const [structureLoading, setStructureLoading] = useState(false);
  const [structureError, setStructureError] = useState("");
  const [showAddColumnModal, setShowAddColumnModal] = useState(false);
  const [newColName, setNewColName] = useState("");
  const [newColType, setNewColType] = useState("VARCHAR(255)");
  const [newColNullable, setNewColNullable] = useState(true);
  const [newColDefault, setNewColDefault] = useState("");
  const [showCreateTableModal, setShowCreateTableModal] = useState(false);
  const [newTableName, setNewTableName] = useState("");

  const loadStructure = useCallback(async () => {
    if (!selectedTable) return;
    setStructureLoading(true);
    setStructureError("");
    try {
      const [cols, idxs] = await Promise.allSettled([
        api.get<QueryResult>(`/databases/${database.id}/tables/${encodeURIComponent(selectedTable)}`),
        api.get<QueryResult>(`/databases/${database.id}/indexes/${encodeURIComponent(selectedTable)}`),
      ]);
      if (cols.status === "fulfilled") setStructureCols(cols.value);
      if (idxs.status === "fulfilled") setStructureIndexes(idxs.value);
    } catch (e) {
      setStructureError(e instanceof Error ? e.message : "Failed to load structure");
    } finally {
      setStructureLoading(false);
    }
  }, [database.id, selectedTable]);

  useEffect(() => {
    if (activeTab === "structure" && selectedTable) {
      loadStructure();
    }
  }, [activeTab, selectedTable, loadStructure]);

  const handleAddColumn = async (e: FormEvent) => {
    e.preventDefault();
    if (!selectedTable || !newColName.trim()) return;
    try {
      let sql = `ALTER TABLE ${q}${selectedTable}${q} ADD COLUMN ${q}${newColName.trim()}${q} ${newColType}`;
      if (!newColNullable) sql += " NOT NULL";
      if (newColDefault.trim()) sql += ` DEFAULT '${newColDefault.trim().replace(/'/g, "''")}'`;
      await api.post(`/databases/${database.id}/query`, { sql });
      showGlobalNotification(`Column "${newColName}" added successfully`, "success");
      setShowAddColumnModal(false);
      setNewColName("");
      setNewColDefault("");
      loadStructure();
    } catch (err) {
      showGlobalNotification(err instanceof Error ? err.message : "Failed to add column", "error");
    }
  };

  const handleDropColumn = (colName: string) => {
    if (!selectedTable) return;
    setConfirmModal({
      title: `Drop Column: ${colName}`,
      label: `Are you sure you want to drop column "${colName}" from table "${selectedTable}"?`,
      consequences: [
        `All data in column "${colName}" across all rows will be permanently erased`,
        "Any queries or software depending on this column will fail",
      ],
      confirmLabel: "Drop Column",
      onConfirm: async () => {
        try {
          const sql = `ALTER TABLE ${q}${selectedTable}${q} DROP COLUMN ${q}${colName}${q}`;
          await api.post(`/databases/${database.id}/query`, { sql });
          showGlobalNotification(`Column "${colName}" dropped successfully`, "success");
          loadStructure();
        } catch (err) {
          showGlobalNotification(err instanceof Error ? err.message : "Failed to drop column", "error");
        } finally {
          setConfirmModal(null);
        }
      },
    });
  };

  const handleCreateTable = async (e: FormEvent) => {
    e.preventDefault();
    if (!newTableName.trim()) return;
    try {
      const sql =
        database.engine === "postgres"
          ? `CREATE TABLE ${q}${newTableName.trim()}${q} (${q}id${q} SERIAL PRIMARY KEY, ${q}created_at${q} TIMESTAMP DEFAULT CURRENT_TIMESTAMP)`
          : `CREATE TABLE ${q}${newTableName.trim()}${q} (${q}id${q} INT AUTO_INCREMENT PRIMARY KEY, ${q}created_at${q} TIMESTAMP DEFAULT CURRENT_TIMESTAMP) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`;
      await api.post(`/databases/${database.id}/query`, { sql });
      showGlobalNotification(`Table "${newTableName}" created successfully`, "success");
      setShowCreateTableModal(false);
      const createdName = newTableName.trim();
      setNewTableName("");
      await loadTables();
      setSelectedTable(createdName);
    } catch (err) {
      showGlobalNotification(err instanceof Error ? err.message : "Failed to create table", "error");
    }
  };

  /* ──────────────── SQL Console Tab State ──────────────── */
  const [sqlQuery, setSqlQuery] = useState("");
  const [sqlResult, setSqlResult] = useState<QueryResult | null>(null);
  const [sqlLoading, setSqlLoading] = useState(false);
  const [sqlError, setSqlError] = useState("");
  const [sqlHistory, setSqlHistory] = useState<string[]>([]);
  const [sqlHistoryIdx, setSqlHistoryIdx] = useState(-1);
  const sqlEditorRef = useRef<HTMLTextAreaElement>(null);

  const executeSqlQuery = useCallback(
    async (rawSql?: string) => {
      const toRun = rawSql || sqlQuery;
      if (!toRun.trim()) return;
      setSqlLoading(true);
      setSqlError("");
      setSqlResult(null);
      try {
        const res = await api.post<QueryResult>(`/databases/${database.id}/query`, { sql: toRun });
        setSqlResult(res);
        setSqlHistory((prev) => [toRun, ...prev.filter((h) => h !== toRun)].slice(0, 50));
        setSqlHistoryIdx(-1);
      } catch (e) {
        setSqlError(e instanceof Error ? e.message : "Query execution failed");
      } finally {
        setSqlLoading(false);
      }
    },
    [database.id, sqlQuery]
  );

  const handleSqlKeyDown = (e: React.KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      executeSqlQuery();
    }
    if (e.ctrlKey && e.key === "ArrowUp" && sqlHistory.length > 0) {
      e.preventDefault();
      const next = Math.min(sqlHistoryIdx + 1, sqlHistory.length - 1);
      setSqlHistoryIdx(next);
      setSqlQuery(sqlHistory[next]);
    }
    if (e.ctrlKey && e.key === "ArrowDown" && sqlHistoryIdx > 0) {
      e.preventDefault();
      const next = sqlHistoryIdx - 1;
      setSqlHistoryIdx(next);
      setSqlQuery(sqlHistory[next]);
    }
  };

  /* ──────────────── Import & Upload Tab State ──────────────── */
  const [dumps, setDumps] = useState<DumpFile[]>([]);
  const [dumpsLoading, setDumpsLoading] = useState(false);
  const [dumpsError, setDumpsError] = useState("");
  const [importingDump, setImportingDump] = useState<string | null>(null);
  const [uploadProgress, setUploadProgress] = useState<number | null>(null);
  const [uploadStatus, setUploadStatus] = useState("");
  const [autoImportUpload, setAutoImportUpload] = useState(true);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const loadDumps = useCallback(async () => {
    setDumpsLoading(true);
    setDumpsError("");
    try {
      const list = await api.get<DumpFile[]>(`/databases/${database.id}/dumps`);
      setDumps(Array.isArray(list) ? list : []);
    } catch (e) {
      setDumpsError(e instanceof Error ? e.message : "Failed to load dumps");
    } finally {
      setDumpsLoading(false);
    }
  }, [database.id]);

  useEffect(() => {
    if (activeTab === "import" || activeTab === "export") {
      loadDumps();
    }
  }, [activeTab, loadDumps]);

  const handleUploadDumpFile = async (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    if (!file.name.endsWith(".sql") && !file.name.endsWith(".sql.gz") && !file.name.endsWith(".gz")) {
      showGlobalNotification("Please select a valid .sql or .sql.gz dump file", "error");
      return;
    }

    setUploadProgress(10);
    setUploadStatus(`Reading ${file.name} (${formatBytes(file.size)})...`);

    const reader = new FileReader();
    reader.onprogress = (evt) => {
      if (evt.lengthComputable) {
        const pct = Math.round((evt.loaded / evt.total) * 40) + 10;
        setUploadProgress(pct);
      }
    };

    reader.onload = async () => {
      setUploadProgress(50);
      setUploadStatus("Encoding and uploading dump to server...");
      try {
        const resultStr = reader.result as string;
        const base64Index = resultStr.indexOf("base64,");
        const base64Content = base64Index !== -1 ? resultStr.substring(base64Index + 7) : resultStr;

        setUploadProgress(70);
        setUploadStatus(autoImportUpload ? "Uploading and restoring into database container..." : "Saving dump on server...");

        await api.post(`/databases/${database.id}/upload-dump`, {
          filename: file.name,
          content_base64: base64Content,
          auto_import: autoImportUpload,
        });

        setUploadProgress(100);
        setUploadStatus("Finished!");
        showGlobalNotification(
          autoImportUpload
            ? `Successfully uploaded and restored "${file.name}" into ${database.name}!`
            : `Dump "${file.name}" uploaded successfully!`,
          "success"
        );
        loadDumps();
        loadTables();
      } catch (err) {
        showGlobalNotification(err instanceof Error ? err.message : "Upload failed", "error");
      } finally {
        setTimeout(() => {
          setUploadProgress(null);
          setUploadStatus("");
          if (fileInputRef.current) fileInputRef.current.value = "";
        }, 2000);
      }
    };

    reader.onerror = () => {
      showGlobalNotification("Failed to read file from disk", "error");
      setUploadProgress(null);
      setUploadStatus("");
    };

    reader.readAsDataURL(file);
  };

  const handleImportDump = (filename: string) => {
    setConfirmModal({
      title: `Restore Database: ${database.name}`,
      label: `Are you sure you want to restore dump "${filename}" into "${database.name}"?`,
      consequences: [
        `All existing tables and records in "${database.name}" will be completely replaced by the dump`,
        "Any uncommitted or recent changes since the backup was made will be permanently lost",
        "The database container will be temporarily locked while the dump is being imported",
      ],
      confirmLabel: "Restore Database",
      onConfirm: async () => {
        setImportingDump(filename);
        try {
          await api.post(`/databases/${database.id}/import`, { filename });
          showGlobalNotification(`Dump "${filename}" restored into ${database.name} successfully`, "success");
          loadTables();
          loadBrowseData();
        } catch (e) {
          showGlobalNotification(e instanceof Error ? e.message : "Import failed", "error");
        } finally {
          setImportingDump(null);
          setConfirmModal(null);
        }
      },
    });
  };

  /* ──────────────── Export Tab State ──────────────── */
  const [exporting, setExporting] = useState(false);

  const handleCreateExport = async () => {
    setExporting(true);
    try {
      const res = await api.post<{ ok: boolean; filename: string }>(`/databases/${database.id}/export`, {});
      showGlobalNotification(`Export "${res.filename}" generated successfully!`, "success");
      loadDumps();
    } catch (e) {
      showGlobalNotification(e instanceof Error ? e.message : "Export failed", "error");
    } finally {
      setExporting(false);
    }
  };

  /* ──────────────── Find & Replace Tab State ──────────────── */
  const [findTable, setFindTable] = useState<string>("");
  const [findColumn, setFindColumn] = useState<string>("");
  const [findText, setFindText] = useState("");
  const [replaceText, setReplaceText] = useState("");
  const [findColumnsList, setFindColumnsList] = useState<string[]>([]);
  const [previewMatchesCount, setPreviewMatchesCount] = useState<number | null>(null);
  const [findLoading, setFindLoading] = useState(false);

  useEffect(() => {
    if (findTable) {
      (async () => {
        try {
          const res = await api.get<QueryResult>(`/databases/${database.id}/tables/${encodeURIComponent(findTable)}`);
          if (res.rows) {
            setFindColumnsList(res.rows.map((r) => r[0]));
            if (res.rows.length > 0) setFindColumn(res.rows[0][0]);
          }
        } catch {
          // Ignored
        }
      })();
    }
  }, [database.id, findTable]);

  useEffect(() => {
    if (tables.length > 0 && !findTable) {
      setFindTable(tables[0]);
    }
  }, [tables, findTable]);

  const handlePreviewFind = async () => {
    if (!findTable || !findColumn || !findText) return;
    setFindLoading(true);
    setPreviewMatchesCount(null);
    try {
      const sql = `SELECT COUNT(*) FROM ${q}${findTable}${q} WHERE ${q}${findColumn}${q} LIKE '%${findText.replace(/'/g, "''")}%'`;
      const res = await api.post<QueryResult>(`/databases/${database.id}/query`, { sql });
      if (res.rows?.[0]?.[0]) {
        setPreviewMatchesCount(parseInt(res.rows[0][0], 10));
      }
    } catch (e) {
      showGlobalNotification(e instanceof Error ? e.message : "Preview count failed", "error");
    } finally {
      setFindLoading(false);
    }
  };

  const handleExecuteReplace = () => {
    if (!findTable || !findColumn || !findText) return;
    setConfirmModal({
      title: `Find & Replace in Table: ${findTable}`,
      label: `Replace all occurrences of "${findText}" with "${replaceText}" in column "${findColumn}"?`,
      consequences: [
        `Directly modifies all matching rows in column "${findColumn}" of table "${findTable}"`,
        "This update cannot be undone automatically — ensure you have an export if needed",
      ],
      confirmLabel: "Execute Replace",
      onConfirm: async () => {
        setFindLoading(true);
        try {
          const sql = `UPDATE ${q}${findTable}${q} SET ${q}${findColumn}${q} = REPLACE(${q}${findColumn}${q}, '${findText.replace(/'/g, "''")}', '${replaceText.replace(/'/g, "''")}') WHERE ${q}${findColumn}${q} LIKE '%${findText.replace(/'/g, "''")}%'`;
          await api.post(`/databases/${database.id}/query`, { sql });
          showGlobalNotification("Find & Replace completed successfully", "success");
          setPreviewMatchesCount(null);
          loadBrowseData();
        } catch (e) {
          showGlobalNotification(e instanceof Error ? e.message : "Replace failed", "error");
        } finally {
          setFindLoading(false);
          setConfirmModal(null);
        }
      },
    });
  };

  return (
    <div className="animate-fade-up">
      {/* Lethal Confirmation Dialog */}
      {confirmModal && (
        <ConfirmDialog
          title={confirmModal.title}
          label={confirmModal.label}
          consequences={confirmModal.consequences}
          confirmLabel={confirmModal.confirmLabel || "Confirm"}
          tone="danger"
          onConfirm={confirmModal.onConfirm}
          onCancel={() => setConfirmModal(null)}
        />
      )}

      {/* Top Header Bar */}
      <div className="flex flex-wrap items-center justify-between gap-4 mb-5 pb-4 border-b border-dark-600">
        <div className="flex items-center gap-3">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-sm text-dark-300 border border-dark-600 rounded-lg hover:text-dark-100 hover:border-dark-400 transition-colors"
          >
            &larr; Back to Databases
          </button>
          <div>
            <div className="flex items-center gap-2">
              <h1 className="text-base font-semibold text-dark-50 font-mono">{database.name}</h1>
              <span className="px-2 py-0.5 bg-dark-700 text-dark-200 rounded text-xs font-medium">
                {engineLabels[database.engine] || database.engine}
              </span>
              {database.port && (
                <span className="text-xs text-dark-400 font-mono">Port {database.port}</span>
              )}
            </div>
            <p className="text-xs text-dark-400 mt-0.5">phpMyAdmin-grade SQL Management Suite</p>
          </div>
        </div>

        {/* Tab Navigation */}
        <div className="flex flex-wrap items-center gap-1 bg-dark-800 p-1 rounded-lg border border-dark-600">
          {[
            { id: "browse", label: "Browse Data", icon: "M3.375 19.5h17.25m-17.25 0a1.125 1.125 0 0 1-1.125-1.125M3.375 19.5h7.5c.621 0 1.125-.504 1.125-1.125m-9.75 0V5.625m0 12.75v-1.5c0-.621.504-1.125 1.125-1.125m18.375 2.625V5.625m0 12.75c0 .621-.504 1.125-1.125 1.125m1.125-1.125v-1.5c0-.621-.504-1.125-1.125-1.125m0 3.75h-7.5A1.125 1.125 0 0 1 12 18.375m9.75-12.75c0-.621-.504-1.125-1.125-1.125H3.375c-.621 0-1.125.504-1.125 1.125m19.5 0v1.5c0 .621-.504 1.125-1.125 1.125M2.25 5.625v1.5c0 .621.504 1.125 1.125 1.125m0 0h17.25m-17.25 0h7.5c.621 0 1.125.504 1.125 1.125M3.375 8.25c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125m17.25-3.75h-7.5c-.621 0-1.125.504-1.125 1.125m8.625-1.125c.621 0 1.125.504 1.125 1.125v1.5c0 .621-.504 1.125-1.125 1.125m-17.25 0h17.25m-17.25 0h7.5c.621 0 1.125.504 1.125 1.125M3.375 12c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125m17.25-3.75h-7.5c-.621 0-1.125.504-1.125 1.125m8.625-1.125c.621 0 1.125.504 1.125 1.125v1.5c0 .621-.504 1.125-1.125 1.125m-17.25 0h17.25" },
            { id: "structure", label: "Structure", icon: "M3.75 6A2.25 2.25 0 0 1 6 3.75h2.25A2.25 2.25 0 0 1 10.5 6v2.25a2.25 2.25 0 0 1-2.25 2.25H6a2.25 2.25 0 0 1-2.25-2.25V6ZM3.75 15.75A2.25 2.25 0 0 1 6 13.5h2.25a2.25 2.25 0 0 1 2.25 2.25V18a2.25 2.25 0 0 1-2.25 2.25H6A2.25 2.25 0 0 1 3.75 18v-2.25ZM13.5 6a2.25 2.25 0 0 1 2.25-2.25H18A2.25 2.25 0 0 1 20.25 6v2.25A2.25 2.25 0 0 1 18 10.5h-2.25a2.25 2.25 0 0 1-2.25-2.25V6ZM13.5 15.75a2.25 2.25 0 0 1 2.25-2.25H18a2.25 2.25 0 0 1 2.25 2.25V18A2.25 2.25 0 0 1 18 20.25h-2.25A2.25 2.25 0 0 1 13.5 18v-2.25Z" },
            { id: "sql", label: "SQL Console", icon: "M6.75 7.5l3 2.25-3 2.25m4.5 0h3m-9 8.25h13.5A2.25 2.25 0 0 0 21 18V6a2.25 2.25 0 0 0-2.25-2.25H5.25A2.25 2.25 0 0 0 3 6v12a2.25 2.25 0 0 0 2.25 2.25Z" },
            { id: "import", label: "Import & Upload", icon: "M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.5V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" },
            { id: "export", label: "Export & Backups", icon: "M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.5V16.5M16.5 12 12 16.5m0 0L7.5 12m4.5 4.5V3" },
            { id: "find_replace", label: "Find & Replace", icon: "M21 21l-5.197-5.197m0 0A7.5 7.5 0 1 0 5.196 5.196a7.5 7.5 0 0 0 10.607 10.607ZM10.5 7.5v6m3-3h-6" },
          ].map((t) => (
            <button
              key={t.id}
              onClick={() => setActiveTab(t.id as ManagerTab)}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors ${
                activeTab === t.id
                  ? "bg-rust-500 text-white shadow-sm"
                  : "text-dark-300 hover:text-dark-100 hover:bg-dark-700/60"
              }`}
            >
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
                <path strokeLinecap="round" strokeLinejoin="round" d={t.icon} />
              </svg>
              {t.label}
            </button>
          ))}
        </div>
      </div>

      {/* Main Container */}
      <div className="flex gap-4" style={{ minHeight: "calc(100vh - 240px)" }}>
        {/* Left Sidebar (Only for Browse & Structure) */}
        {(activeTab === "browse" || activeTab === "structure") && (
          <div className="w-60 shrink-0 bg-dark-800 rounded-lg border border-dark-600 overflow-hidden flex flex-col">
            <div className="px-3.5 py-3 border-b border-dark-600 flex items-center justify-between">
              <span className="text-xs font-semibold text-dark-300 uppercase font-mono tracking-wider">
                Tables ({tables.length})
              </span>
              <div className="flex items-center gap-1.5">
                <button
                  onClick={() => setShowCreateTableModal(true)}
                  className="p-1 rounded hover:bg-dark-700 text-rust-400 hover:text-rust-300 transition-colors"
                  title="Create new table"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
                  </svg>
                </button>
                <button
                  onClick={loadTables}
                  className="p-1 rounded hover:bg-dark-700 text-dark-400 hover:text-dark-200 transition-colors"
                  title="Refresh tables"
                >
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0 3.181 3.183a8.25 8.25 0 0 0 13.803-3.7M4.031 9.865a8.25 8.25 0 0 1 13.803-3.7l3.181 3.182" />
                  </svg>
                </button>
              </div>
            </div>

            <div className="overflow-y-auto flex-1 p-1 space-y-0.5">
              {tablesLoading ? (
                <div className="p-3 space-y-2">
                  {[...Array(6)].map((_, i) => (
                    <div key={i} className="h-6 bg-dark-700/60 rounded animate-pulse" />
                  ))}
                </div>
              ) : tables.length === 0 ? (
                <div className="text-center py-8 text-xs text-dark-400">
                  No tables found.<br />
                  <button
                    onClick={() => setShowCreateTableModal(true)}
                    className="mt-2 text-rust-400 hover:underline"
                  >
                    + Create table
                  </button>
                </div>
              ) : (
                tables.map((t) => {
                  const detail = tableDetails[t];
                  const isSelected = selectedTable === t;
                  return (
                    <button
                      key={t}
                      onClick={() => {
                        setSelectedTable(t);
                        setBrowsePage(0);
                      }}
                      className={`w-full text-left px-2.5 py-2 rounded text-xs font-mono transition-colors flex items-center justify-between ${
                        isSelected
                          ? "bg-rust-500/15 text-rust-400 font-semibold border border-rust-500/30"
                          : "text-dark-200 hover:bg-dark-700 hover:text-dark-50"
                      }`}
                    >
                      <span className="truncate">{t}</span>
                      {detail && (
                        <span className="text-[10px] text-dark-400 font-mono shrink-0 ml-1.5">
                          {detail.rows}r
                        </span>
                      )}
                    </button>
                  );
                })
              )}
            </div>
          </div>
        )}

        {/* Right Active Tab Content */}
        <div className="flex-1 min-w-0">
          {/* TAB 1: BROWSE DATA */}
          {activeTab === "browse" && (
            <div className="space-y-4">
              {selectedTable ? (
                <>
                  {/* Action & Filter Bar */}
                  <div className="bg-dark-800 rounded-lg border border-dark-600 p-3.5 flex flex-wrap items-center justify-between gap-3">
                    <div className="flex items-center gap-3">
                      <span className="text-sm font-semibold text-dark-100 font-mono">{selectedTable}</span>
                      {totalRows !== null && (
                        <span className="text-xs text-dark-400 font-mono">
                          ({totalRows.toLocaleString()} total rows)
                        </span>
                      )}
                    </div>

                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => {
                          setInsertColValues({});
                          setShowInsertModal(true);
                        }}
                        className="px-2.5 py-1.5 bg-rust-500 text-white rounded text-xs font-medium hover:bg-rust-600 transition-colors flex items-center gap-1.5"
                      >
                        <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
                        </svg>
                        Insert Row
                      </button>
                      <button
                        onClick={() => handleTruncateTable(selectedTable)}
                        className="px-2.5 py-1.5 bg-warn-500/10 text-warn-400 hover:bg-warn-500/20 border border-warn-500/20 rounded text-xs font-medium transition-colors"
                        title="Empty all rows"
                      >
                        Empty (Truncate)
                      </button>
                      <button
                        onClick={() => handleDropTable(selectedTable)}
                        className="px-2.5 py-1.5 bg-danger-500/10 text-danger-400 hover:bg-danger-500/20 border border-danger-500/20 rounded text-xs font-medium transition-colors"
                        title="Permanently drop table"
                      >
                        Drop Table
                      </button>
                    </div>
                  </div>

                  {/* Filter WHERE Query */}
                  <div className="bg-dark-800 rounded-lg border border-dark-600 p-3 flex flex-wrap items-center gap-2">
                    <span className="text-xs font-mono text-dark-400 uppercase font-semibold">WHERE</span>
                    <input
                      type="text"
                      value={whereClause}
                      onChange={(e) => setWhereClause(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          setBrowsePage(0);
                          loadBrowseData();
                        }
                      }}
                      placeholder="e.g. status = 'active' OR id > 100"
                      className="flex-1 min-w-[200px] bg-dark-900 border border-dark-600 rounded px-2.5 py-1 text-xs font-mono text-dark-100 placeholder-dark-500 focus:outline-none focus:border-dark-400"
                    />
                    <button
                      onClick={() => {
                        setBrowsePage(0);
                        loadBrowseData();
                      }}
                      className="px-3 py-1 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
                    >
                      Filter
                    </button>
                    {whereClause && (
                      <button
                        onClick={() => {
                          setWhereClause("");
                          setBrowsePage(0);
                        }}
                        className="px-2 py-1 text-dark-400 hover:text-dark-200 text-xs"
                      >
                        Clear
                      </button>
                    )}

                    <div className="h-4 w-px bg-dark-600 mx-1 hidden sm:block" />

                    {/* Page Size Selector */}
                    <div className="flex items-center gap-1.5 ml-auto text-xs text-dark-400 font-mono">
                      <span>Show:</span>
                      <select
                        value={pageSize}
                        onChange={(e) => {
                          setPageSize(Number(e.target.value));
                          setBrowsePage(0);
                        }}
                        className="bg-dark-900 border border-dark-600 rounded px-2 py-1 text-xs text-dark-200 focus:outline-none"
                      >
                        <option value={10}>10</option>
                        <option value={25}>25</option>
                        <option value={50}>50</option>
                        <option value={100}>100</option>
                      </select>
                    </div>
                  </div>

                  {/* Error banner */}
                  {browseError && (
                    <div className="bg-danger-500/10 text-danger-400 text-xs p-3 rounded-lg border border-danger-500/20">
                      {browseError}
                    </div>
                  )}

                  {/* Data Grid Table */}
                  <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-x-auto">
                    {browseLoading ? (
                      <div className="p-8 text-center text-dark-400 text-xs flex items-center justify-center gap-2">
                        <span className="w-4 h-4 border-2 border-rust-500 border-t-transparent rounded-full animate-spin" />
                        Fetching table rows...
                      </div>
                    ) : !browseRows || browseRows.rows.length === 0 ? (
                      <div className="p-12 text-center text-dark-400 text-xs">
                        No rows found matching current query or table is empty.
                      </div>
                    ) : (
                      <table className="w-full text-left">
                        <thead>
                          <tr className="border-b border-dark-600 bg-dark-900/80 sticky top-0">
                            <th className="px-3 py-2 text-[10px] text-dark-400 uppercase font-mono w-10">#</th>
                            {browseRows.columns.map((col) => (
                              <th
                                key={col}
                                onClick={() => handleSort(col)}
                                className="px-3 py-2 text-xs font-semibold text-dark-200 font-mono whitespace-nowrap cursor-pointer hover:text-rust-400 select-none"
                              >
                                <div className="flex items-center gap-1">
                                  <span>{col}</span>
                                  {sortCol === col && (
                                    <span className="text-rust-400">{sortDir === "ASC" ? "▲" : "▼"}</span>
                                  )}
                                </div>
                              </th>
                            ))}
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-dark-700/60 font-mono text-xs">
                          {browseRows.rows.map((row, rIdx) => (
                            <tr key={rIdx} className="hover:bg-dark-700/30 transition-colors">
                              <td className="px-3 py-2 text-[10px] text-dark-500 font-mono">
                                {browsePage * pageSize + rIdx + 1}
                              </td>
                              {row.map((val, cIdx) => (
                                <td
                                  key={cIdx}
                                  className="px-3 py-2 text-dark-200 whitespace-nowrap max-w-sm truncate"
                                  title={val}
                                >
                                  {val === "" || val === "\N" ? (
                                    <span className="text-dark-500 italic text-[10px]">NULL</span>
                                  ) : (
                                    val
                                  )}
                                </td>
                              ))}
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    )}

                    {/* Pagination Bar */}
                    {browseRows && (
                      <div className="p-3 border-t border-dark-600 bg-dark-900/60 flex items-center justify-between text-xs text-dark-300 font-mono">
                        <div>
                          Showing {browseRows.rows.length} rows &middot; Page {browsePage + 1}
                          {browseRows.execution_time_ms ? ` (${browseRows.execution_time_ms}ms)` : ""}
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => setBrowsePage((p) => Math.max(0, p - 1))}
                            disabled={browsePage === 0 || browseLoading}
                            className="px-2.5 py-1 rounded bg-dark-700 text-dark-200 hover:bg-dark-600 disabled:opacity-40 transition-colors"
                          >
                            &larr; Prev
                          </button>
                          <button
                            onClick={() => setBrowsePage((p) => p + 1)}
                            disabled={browseRows.rows.length < pageSize || browseLoading}
                            className="px-2.5 py-1 rounded bg-dark-700 text-dark-200 hover:bg-dark-600 disabled:opacity-40 transition-colors"
                          >
                            Next &rarr;
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                </>
              ) : (
                <div className="bg-dark-800 rounded-lg border border-dark-600 p-12 text-center text-dark-400 text-sm">
                  Select a table from the sidebar to browse its rows and records.
                </div>
              )}
            </div>
          )}

          {/* TAB 2: STRUCTURE */}
          {activeTab === "structure" && (
            <div className="space-y-4">
              {selectedTable ? (
                <>
                  <div className="bg-dark-800 rounded-lg border border-dark-600 p-4 flex flex-wrap items-center justify-between gap-3">
                    <div>
                      <h2 className="text-sm font-semibold text-dark-100 font-mono">{selectedTable} Schema</h2>
                      <p className="text-xs text-dark-400 mt-0.5">Columns, indexes and relational foreign keys</p>
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => setShowAddColumnModal(true)}
                        className="px-3 py-1.5 bg-rust-500 text-white rounded text-xs font-medium hover:bg-rust-600 transition-colors flex items-center gap-1.5"
                      >
                        <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
                        </svg>
                        Add Column
                      </button>
                      <button
                        onClick={loadStructure}
                        className="px-2.5 py-1.5 bg-dark-700 text-dark-200 hover:bg-dark-600 rounded text-xs font-medium transition-colors"
                      >
                        Refresh
                      </button>
                    </div>
                  </div>

                  {structureError && (
                    <div className="bg-danger-500/10 text-danger-400 text-xs p-3 rounded-lg border border-danger-500/20">
                      {structureError}
                    </div>
                  )}

                  {/* Columns Table */}
                  <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-hidden">
                    <div className="px-4 py-2.5 border-b border-dark-600 bg-dark-900/60 flex items-center justify-between">
                      <span className="text-xs uppercase font-mono tracking-wider text-dark-300 font-semibold">
                        Columns ({structureCols?.rows?.length || 0})
                      </span>
                    </div>
                    {structureLoading ? (
                      <div className="p-6 text-center text-xs text-dark-400">Loading column schema...</div>
                    ) : !structureCols || structureCols.rows.length === 0 ? (
                      <div className="p-6 text-center text-xs text-dark-400">No columns found.</div>
                    ) : (
                      <table className="w-full text-left text-xs font-mono">
                        <thead>
                          <tr className="border-b border-dark-600 bg-dark-700/40 text-dark-300">
                            <th className="px-4 py-2 uppercase">Column</th>
                            <th className="px-4 py-2 uppercase">Type</th>
                            <th className="px-4 py-2 uppercase">Nullable</th>
                            <th className="px-4 py-2 uppercase">Default</th>
                            <th className="px-4 py-2 uppercase">Key</th>
                            <th className="px-4 py-2 uppercase text-right">Actions</th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-dark-700/60">
                          {structureCols.rows.map((col, idx) => {
                            const colName = col[0];
                            const colType = col[1];
                            const isNull = database.engine === "postgres" ? col[3] : col[2];
                            const defVal = database.engine === "postgres" ? col[4] : col[3];
                            const keyVal = database.engine === "postgres" ? "" : col[4];
                            return (
                              <tr key={idx} className="hover:bg-dark-700/30 transition-colors">
                                <td className="px-4 py-2.5 font-semibold text-dark-100">{colName}</td>
                                <td className="px-4 py-2.5 text-rust-400">{colType}</td>
                                <td className="px-4 py-2.5 text-dark-300">{isNull === "YES" ? "Yes" : "No"}</td>
                                <td className="px-4 py-2.5 text-dark-400">{defVal || "-"}</td>
                                <td className="px-4 py-2.5 text-accent-400">{keyVal || "-"}</td>
                                <td className="px-4 py-2.5 text-right">
                                  <button
                                    onClick={() => handleDropColumn(colName)}
                                    className="text-danger-400 hover:text-danger-300 text-xs"
                                    title="Drop Column"
                                  >
                                    Drop
                                  </button>
                                </td>
                              </tr>
                            );
                          })}
                        </tbody>
                      </table>
                    )}
                  </div>

                  {/* Indexes Table */}
                  {structureIndexes && structureIndexes.rows.length > 0 && (
                    <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-hidden">
                      <div className="px-4 py-2.5 border-b border-dark-600 bg-dark-900/60">
                        <span className="text-xs uppercase font-mono tracking-wider text-dark-300 font-semibold">
                          Indexes
                        </span>
                      </div>
                      <table className="w-full text-left text-xs font-mono">
                        <thead>
                          <tr className="border-b border-dark-600 bg-dark-700/40 text-dark-300">
                            {structureIndexes.columns.map((c, i) => (
                              <th key={i} className="px-4 py-2 uppercase">{c}</th>
                            ))}
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-dark-700/60">
                          {structureIndexes.rows.map((row, i) => (
                            <tr key={i} className="hover:bg-dark-700/30">
                              {row.map((cell, j) => (
                                <td key={j} className="px-4 py-2 text-dark-200">{cell}</td>
                              ))}
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  )}

                  {/* Foreign Keys / Relations */}
                  {(() => {
                    const srcTableIdx = fkColumns.indexOf("source_table");
                    const srcColIdx = fkColumns.indexOf("source_column");
                    const tgtTableIdx = fkColumns.indexOf("target_table");
                    const tgtColIdx = fkColumns.indexOf("target_column");
                    const related = foreignKeys.filter(
                      (fk) => fk[srcTableIdx] === selectedTable || fk[tgtTableIdx] === selectedTable
                    );
                    if (related.length === 0) return null;
                    return (
                      <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-hidden">
                        <div className="px-4 py-2.5 border-b border-dark-600 bg-dark-900/60">
                          <span className="text-xs uppercase font-mono tracking-wider text-dark-300 font-semibold">
                            Foreign Key Relationships
                          </span>
                        </div>
                        <div className="p-4 space-y-2">
                          {related.map((fk, idx) => (
                            <div key={idx} className="flex items-center gap-2 text-xs font-mono">
                              <span className="text-dark-100 font-semibold">{fk[srcTableIdx]}</span>
                              <span className="text-dark-400">.</span>
                              <span className="text-rust-400">{fk[srcColIdx]}</span>
                              <span className="text-dark-500">&rarr;</span>
                              <span className="text-dark-100 font-semibold">{fk[tgtTableIdx]}</span>
                              <span className="text-dark-400">.</span>
                              <span className="text-rust-400">{fk[tgtColIdx]}</span>
                            </div>
                          ))}
                        </div>
                      </div>
                    );
                  })()}
                </>
              ) : (
                <div className="bg-dark-800 rounded-lg border border-dark-600 p-12 text-center text-dark-400 text-sm">
                  Select a table from the sidebar to inspect its structure.
                </div>
              )}
            </div>
          )}

          {/* TAB 3: SQL CONSOLE */}
          {activeTab === "sql" && (
            <div className="space-y-4">
              <div className="bg-dark-800 rounded-lg border border-dark-600 p-4 space-y-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <span className="text-xs uppercase font-mono tracking-wider text-dark-300 font-semibold">
                      Query Editor
                    </span>
                    <span className="text-[10px] text-dark-500 font-mono">(Ctrl+Enter to Run)</span>
                  </div>

                  {/* Preset Query Snippets */}
                  <div className="flex flex-wrap items-center gap-1.5">
                    {[
                      { label: "SELECT *", qStr: selectedTable ? `SELECT * FROM ${q}${selectedTable}${q} LIMIT 50;` : `SELECT * FROM ... LIMIT 50;` },
                      { label: "SHOW TABLES", qStr: database.engine === "postgres" ? "SELECT tablename FROM pg_tables WHERE schemaname = 'public';" : "SHOW TABLES;" },
                      { label: "PROCESSLIST", qStr: database.engine === "postgres" ? "SELECT * FROM pg_stat_activity;" : "SHOW PROCESSLIST;" },
                      { label: "STATUS", qStr: database.engine === "postgres" ? "SELECT * FROM pg_stat_database;" : "SHOW TABLE STATUS;" },
                    ].map((snip, i) => (
                      <button
                        key={i}
                        type="button"
                        onClick={() => setSqlQuery(snip.qStr)}
                        className="px-2 py-0.5 rounded text-[11px] font-mono bg-dark-700 text-dark-300 hover:text-dark-100 hover:bg-dark-600 transition-colors"
                      >
                        {snip.label}
                      </button>
                    ))}
                  </div>
                </div>

                <textarea
                  ref={sqlEditorRef}
                  value={sqlQuery}
                  onChange={(e) => setSqlQuery(e.target.value)}
                  onKeyDown={handleSqlKeyDown}
                  className="w-full h-36 bg-dark-900 border border-dark-600 rounded-lg p-3 text-xs font-mono text-dark-100 resize-y focus:outline-none focus:border-rust-500 placeholder-dark-600"
                  placeholder="SELECT * FROM table WHERE ... ;"
                  spellCheck={false}
                />

                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => executeSqlQuery()}
                      disabled={sqlLoading || !sqlQuery.trim()}
                      className="px-4 py-2 bg-rust-500 text-white rounded text-xs font-semibold hover:bg-rust-600 disabled:opacity-50 transition-colors flex items-center gap-2"
                    >
                      {sqlLoading && (
                        <span className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                      )}
                      Execute Query
                    </button>
                    {sqlHistory.length > 0 && (
                      <span className="text-[10px] text-dark-400 font-mono hidden sm:inline">
                        Ctrl+Up/Down for history
                      </span>
                    )}
                  </div>

                  {sqlResult && (
                    <span className="text-xs text-dark-400 font-mono">
                      {sqlResult.execution_time_ms}ms &middot; {sqlResult.row_count} row{sqlResult.row_count !== 1 ? "s" : ""}
                    </span>
                  )}
                </div>
              </div>

              {/* Error Output */}
              {sqlError && (
                <div className="bg-danger-500/10 text-danger-400 text-xs p-4 rounded-lg border border-danger-500/20 font-mono whitespace-pre-wrap">
                  {sqlError}
                </div>
              )}

              {/* Results Table */}
              {sqlResult && sqlResult.columns.length > 0 && (
                <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-x-auto">
                  <table className="w-full text-left font-mono text-xs">
                    <thead>
                      <tr className="border-b border-dark-600 bg-dark-900/80 sticky top-0">
                        <th className="px-3 py-2 text-[10px] text-dark-500 w-10">#</th>
                        {sqlResult.columns.map((c, i) => (
                          <th key={i} className="px-3 py-2 text-dark-200 font-semibold whitespace-nowrap">
                            {c}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-dark-700/60">
                      {sqlResult.rows.map((row, rIdx) => (
                        <tr key={rIdx} className="hover:bg-dark-700/30">
                          <td className="px-3 py-1.5 text-[10px] text-dark-500">{rIdx + 1}</td>
                          {row.map((cell, cIdx) => (
                            <td key={cIdx} className="px-3 py-1.5 text-dark-200 whitespace-nowrap max-w-sm truncate" title={cell}>
                              {cell === "" || cell === "\N" ? (
                                <span className="text-dark-500 italic text-[10px]">NULL</span>
                              ) : (
                                cell
                              )}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}

              {/* DDL/DML Success */}
              {sqlResult && sqlResult.columns.length === 0 && !sqlError && (
                <div className="bg-dark-800 rounded-lg border border-dark-600 p-8 text-center text-xs text-dark-300 font-mono">
                  Query executed successfully in {sqlResult.execution_time_ms}ms (0 rows returned).
                </div>
              )}
            </div>
          )}

          {/* TAB 4: IMPORT & UPLOAD */}
          {activeTab === "import" && (
            <div className="space-y-5">
              {/* Direct Browser Upload Zone */}
              <div className="bg-dark-800 rounded-lg border border-dashed border-dark-500 p-6 text-center hover:border-rust-500 transition-colors">
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".sql,.sql.gz,.gz"
                  onChange={handleUploadDumpFile}
                  className="hidden"
                  id="db-dump-file-picker"
                />
                <label htmlFor="db-dump-file-picker" className="cursor-pointer block">
                  <div className="w-12 h-12 mx-auto rounded-full bg-rust-500/10 text-rust-400 flex items-center justify-center mb-3">
                    <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.5V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
                    </svg>
                  </div>
                  <h3 className="text-sm font-semibold text-dark-100">Upload Database Dump (.sql or .sql.gz)</h3>
                  <p className="text-xs text-dark-400 mt-1 max-w-md mx-auto">
                    Select a dump file directly from your computer. No need to SSH or SCP into server folders!
                  </p>
                </label>

                {/* Auto Import Checkbox */}
                <div className="mt-4 flex items-center justify-center gap-2">
                  <input
                    type="checkbox"
                    id="auto-import-check"
                    checked={autoImportUpload}
                    onChange={(e) => setAutoImportUpload(e.target.checked)}
                    className="rounded border-dark-600 text-rust-500 focus:ring-0 bg-dark-900"
                  />
                  <label htmlFor="auto-import-check" className="text-xs text-dark-200 cursor-pointer">
                    Automatically restore into database immediately after upload
                  </label>
                </div>

                {/* Live Upload Progress */}
                {uploadProgress !== null && (
                  <div className="mt-5 max-w-md mx-auto space-y-2">
                    <div className="flex justify-between text-xs font-mono text-dark-300">
                      <span>{uploadStatus}</span>
                      <span>{uploadProgress}%</span>
                    </div>
                    <div className="w-full bg-dark-900 rounded-full h-2 overflow-hidden border border-dark-600">
                      <div
                        className="bg-rust-500 h-2 rounded-full transition-all duration-300"
                        style={{ width: `${uploadProgress}%` }}
                      />
                    </div>
                  </div>
                )}
              </div>

              {/* Available Server Dumps */}
              <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-hidden">
                <div className="px-5 py-3.5 border-b border-dark-600 flex items-center justify-between">
                  <div>
                    <h3 className="text-xs font-semibold uppercase font-mono tracking-wider text-dark-300">
                      Available Server Backups & Dumps
                    </h3>
                    <p className="text-[11px] text-dark-400 mt-0.5">Stored in /var/backups/dockpanel/databases/{database.name}</p>
                  </div>
                  <button
                    onClick={loadDumps}
                    className="px-2.5 py-1 text-xs rounded bg-dark-700 text-dark-200 hover:bg-dark-600 transition-colors"
                  >
                    Refresh
                  </button>
                </div>

                {dumpsError && (
                  <div className="p-4 text-xs text-danger-400 bg-danger-500/10 border-b border-danger-500/20">
                    {dumpsError}
                  </div>
                )}

                {dumpsLoading ? (
                  <div className="p-8 text-center text-xs text-dark-400">Loading dumps from server...</div>
                ) : dumps.length === 0 ? (
                  <div className="p-10 text-center text-xs text-dark-400">
                    No dump files found for this database. Use the upload box above or create an export to generate one.
                  </div>
                ) : (
                  <div className="divide-y divide-dark-700/60 font-mono text-xs">
                    {dumps.map((d) => (
                      <div key={d.filename} className="p-4 flex flex-wrap items-center justify-between gap-3 hover:bg-dark-700/20">
                        <div>
                          <div className="font-semibold text-dark-100">{d.filename}</div>
                          <div className="text-[11px] text-dark-400 mt-0.5">
                            {formatBytes(d.size_bytes)} &middot; {d.created_at}
                          </div>
                        </div>

                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => {
                              window.open(`/api/databases/${database.id}/dumps/${encodeURIComponent(d.filename)}/download`, "_blank");
                            }}
                            className="px-2.5 py-1.5 rounded bg-dark-700 text-dark-200 hover:bg-dark-600 transition-colors text-xs font-sans font-medium"
                          >
                            Download
                          </button>
                          <button
                            onClick={() => handleImportDump(d.filename)}
                            disabled={importingDump === d.filename}
                            className="px-3 py-1.5 rounded bg-rust-500 text-white hover:bg-rust-600 disabled:opacity-50 transition-colors text-xs font-sans font-medium"
                          >
                            {importingDump === d.filename ? "Restoring..." : "Restore"}
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}

          {/* TAB 5: EXPORT & BACKUPS */}
          {activeTab === "export" && (
            <div className="space-y-5">
              <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 flex flex-wrap items-center justify-between gap-4">
                <div>
                  <h3 className="text-sm font-semibold text-dark-100 font-mono">Create Fresh Database Dump</h3>
                  <p className="text-xs text-dark-400 mt-1 max-w-xl">
                    Executes a live container dump via mysqldump / pg_dump and packages it as an optimized compressed <code>.sql.gz</code> file ready for 1-click download.
                  </p>
                </div>
                <button
                  onClick={handleCreateExport}
                  disabled={exporting}
                  className="px-4 py-2 bg-rust-500 text-white rounded-lg text-xs font-semibold hover:bg-rust-600 disabled:opacity-50 transition-colors flex items-center gap-2"
                >
                  {exporting && (
                    <span className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  )}
                  {exporting ? "Generating Export..." : "Create Fresh Export (.sql.gz)"}
                </button>
              </div>

              {/* Ready for download list */}
              <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-hidden">
                <div className="px-5 py-3.5 border-b border-dark-600">
                  <h3 className="text-xs font-semibold uppercase font-mono tracking-wider text-dark-300">
                    Exported Dump Files Available for Download
                  </h3>
                </div>

                {dumps.length === 0 ? (
                  <div className="p-10 text-center text-xs text-dark-400">
                    No exported backups yet. Click &quot;Create Fresh Export&quot; above to dump this database.
                  </div>
                ) : (
                  <div className="divide-y divide-dark-700/60 font-mono text-xs">
                    {dumps.map((d) => (
                      <div key={d.filename} className="p-4 flex items-center justify-between gap-3 hover:bg-dark-700/20">
                        <div>
                          <div className="font-semibold text-dark-100">{d.filename}</div>
                          <div className="text-[11px] text-dark-400 mt-0.5">
                            {formatBytes(d.size_bytes)} &middot; {d.created_at}
                          </div>
                        </div>

                        <a
                          href={`/api/databases/${database.id}/dumps/${encodeURIComponent(d.filename)}/download`}
                          download={d.filename}
                          className="px-3 py-1.5 rounded bg-rust-500/15 text-rust-400 hover:bg-rust-500/25 transition-colors text-xs font-sans font-medium flex items-center gap-1.5"
                        >
                          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                            <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.5V16.5M16.5 12 12 16.5m0 0L7.5 12m4.5 4.5V3" />
                          </svg>
                          Download (.sql.gz)
                        </a>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}

          {/* TAB 6: FIND & REPLACE */}
          {activeTab === "find_replace" && (
            <div className="space-y-4">
              <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-4">
                <div>
                  <h3 className="text-sm font-semibold text-dark-100 font-mono">Find & Replace Across Table Data</h3>
                  <p className="text-xs text-dark-400 mt-1">
                    Indispensable for CMS migrations (e.g. updating WordPress URLs, HTTP to HTTPS, or replacing strings in bulk).
                  </p>
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div>
                    <label className="block text-xs font-medium text-dark-200 mb-1">Target Table</label>
                    <select
                      value={findTable}
                      onChange={(e) => setFindTable(e.target.value)}
                      className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-2 text-xs font-mono text-dark-100 focus:outline-none"
                    >
                      {tables.map((t) => (
                        <option key={t} value={t}>{t}</option>
                      ))}
                    </select>
                  </div>

                  <div>
                    <label className="block text-xs font-medium text-dark-200 mb-1">Target Column</label>
                    <select
                      value={findColumn}
                      onChange={(e) => setFindColumn(e.target.value)}
                      className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-2 text-xs font-mono text-dark-100 focus:outline-none"
                    >
                      {findColumnsList.map((c) => (
                        <option key={c} value={c}>{c}</option>
                      ))}
                    </select>
                  </div>

                  <div>
                    <label className="block text-xs font-medium text-dark-200 mb-1">Find String</label>
                    <input
                      type="text"
                      value={findText}
                      onChange={(e) => setFindText(e.target.value)}
                      placeholder="e.g. http://old-domain.com"
                      className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-2 text-xs font-mono text-dark-100 focus:outline-none focus:border-rust-500"
                    />
                  </div>

                  <div>
                    <label className="block text-xs font-medium text-dark-200 mb-1">Replace With</label>
                    <input
                      type="text"
                      value={replaceText}
                      onChange={(e) => setReplaceText(e.target.value)}
                      placeholder="e.g. https://new-domain.com"
                      className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-2 text-xs font-mono text-dark-100 focus:outline-none focus:border-rust-500"
                    />
                  </div>
                </div>

                <div className="flex flex-wrap items-center gap-3 pt-2">
                  <button
                    onClick={handlePreviewFind}
                    disabled={findLoading || !findText || !findTable || !findColumn}
                    className="px-4 py-2 bg-dark-700 text-dark-100 rounded text-xs font-medium hover:bg-dark-600 disabled:opacity-50 transition-colors"
                  >
                    Preview Matches Count
                  </button>
                  <button
                    onClick={handleExecuteReplace}
                    disabled={findLoading || !findText || !findTable || !findColumn}
                    className="px-4 py-2 bg-rust-500 text-white rounded text-xs font-medium hover:bg-rust-600 disabled:opacity-50 transition-colors"
                  >
                    Execute Replace
                  </button>

                  {previewMatchesCount !== null && (
                    <span className="text-xs font-mono text-accent-400">
                      Found {previewMatchesCount} matching row{previewMatchesCount !== 1 ? "s" : ""} in &quot;{findTable}.{findColumn}&quot;
                    </span>
                  )}
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Insert Row Modal */}
      {showInsertModal && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
          <div className="bg-dark-800 rounded-xl border border-dark-600 p-6 w-full max-w-xl max-h-[85vh] overflow-y-auto space-y-4">
            <div className="flex items-center justify-between border-b border-dark-600 pb-3">
              <h3 className="text-sm font-semibold text-dark-100 font-mono">Insert Row into {selectedTable}</h3>
              <button onClick={() => setShowInsertModal(false)} className="text-dark-400 hover:text-dark-200 text-sm">
                &times;
              </button>
            </div>
            <div className="space-y-3">
              {(browseRows?.columns || []).map((col) => (
                <div key={col}>
                  <label className="block text-xs font-mono text-dark-300 mb-1">{col}</label>
                  <input
                    type="text"
                    value={insertColValues[col] || ""}
                    onChange={(e) => setInsertColValues((v) => ({ ...v, [col]: e.target.value }))}
                    placeholder="Leave empty for DEFAULT or auto-increment"
                    className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-1.5 text-xs font-mono text-dark-100 focus:outline-none focus:border-rust-500"
                  />
                </div>
              ))}
            </div>
            <div className="flex justify-end gap-2 pt-3 border-t border-dark-600">
              <button
                type="button"
                onClick={() => setShowInsertModal(false)}
                className="px-3 py-1.5 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={handleInsertRow}
                disabled={inserting}
                className="px-4 py-1.5 bg-rust-500 text-white rounded text-xs font-medium hover:bg-rust-600 disabled:opacity-50"
              >
                {inserting ? "Inserting..." : "Insert Record"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Add Column Modal */}
      {showAddColumnModal && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
          <div className="bg-dark-800 rounded-xl border border-dark-600 p-6 w-full max-w-md space-y-4">
            <div className="flex items-center justify-between border-b border-dark-600 pb-3">
              <h3 className="text-sm font-semibold text-dark-100 font-mono">Add Column to {selectedTable}</h3>
              <button onClick={() => setShowAddColumnModal(false)} className="text-dark-400 hover:text-dark-200 text-sm">
                &times;
              </button>
            </div>
            <form onSubmit={handleAddColumn} className="space-y-3">
              <div>
                <label className="block text-xs font-medium text-dark-200 mb-1">Column Name</label>
                <input
                  type="text"
                  required
                  value={newColName}
                  onChange={(e) => setNewColName(e.target.value)}
                  placeholder="e.g. status, bio, rating"
                  className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-1.5 text-xs font-mono text-dark-100 focus:outline-none focus:border-rust-500"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-dark-200 mb-1">Data Type</label>
                <input
                  type="text"
                  required
                  value={newColType}
                  onChange={(e) => setNewColType(e.target.value)}
                  placeholder="VARCHAR(255), INT, TEXT, TIMESTAMP"
                  className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-1.5 text-xs font-mono text-dark-100 focus:outline-none focus:border-rust-500"
                />
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="checkbox"
                  id="col-nullable-chk"
                  checked={newColNullable}
                  onChange={(e) => setNewColNullable(e.target.checked)}
                  className="rounded border-dark-600 text-rust-500 focus:ring-0 bg-dark-900"
                />
                <label htmlFor="col-nullable-chk" className="text-xs text-dark-200 cursor-pointer">
                  Nullable (Allow NULL)
                </label>
              </div>
              <div>
                <label className="block text-xs font-medium text-dark-200 mb-1">Default Value (Optional)</label>
                <input
                  type="text"
                  value={newColDefault}
                  onChange={(e) => setNewColDefault(e.target.value)}
                  placeholder="Optional default value"
                  className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-1.5 text-xs font-mono text-dark-100 focus:outline-none"
                />
              </div>
              <div className="flex justify-end gap-2 pt-3 border-t border-dark-600">
                <button
                  type="button"
                  onClick={() => setShowAddColumnModal(false)}
                  className="px-3 py-1.5 text-xs text-dark-300 hover:text-dark-100"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-1.5 bg-rust-500 text-white rounded text-xs font-medium hover:bg-rust-600"
                >
                  Add Column
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Create Table Modal */}
      {showCreateTableModal && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
          <div className="bg-dark-800 rounded-xl border border-dark-600 p-6 w-full max-w-md space-y-4">
            <div className="flex items-center justify-between border-b border-dark-600 pb-3">
              <h3 className="text-sm font-semibold text-dark-100 font-mono">Create New Table</h3>
              <button onClick={() => setShowCreateTableModal(false)} className="text-dark-400 hover:text-dark-200 text-sm">
                &times;
              </button>
            </div>
            <form onSubmit={handleCreateTable} className="space-y-3">
              <div>
                <label className="block text-xs font-medium text-dark-200 mb-1">Table Name</label>
                <input
                  type="text"
                  required
                  value={newTableName}
                  onChange={(e) => setNewTableName(e.target.value)}
                  placeholder="e.g. products, customers"
                  pattern="[a-zA-Z0-9_]+"
                  className="w-full bg-dark-900 border border-dark-600 rounded px-3 py-1.5 text-xs font-mono text-dark-100 focus:outline-none focus:border-rust-500"
                />
                <p className="text-[10px] text-dark-400 mt-1">
                  Will be initialized with standard <code>id</code> primary key and <code>created_at</code> timestamp.
                </p>
              </div>
              <div className="flex justify-end gap-2 pt-3 border-t border-dark-600">
                <button
                  type="button"
                  onClick={() => setShowCreateTableModal(false)}
                  className="px-3 py-1.5 text-xs text-dark-300 hover:text-dark-100"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-1.5 bg-rust-500 text-white rounded text-xs font-medium hover:bg-rust-600"
                >
                  Create Table
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
}

/* ─── Main Databases Page ───────────────────────────────────────── */

export default function Databases() {
  const [databases, setDatabases] = useState<Database[]>([]);
  const [sites, setSites] = useState<Site[]>([]);
  const [stacks, setStacks] = useState<Stack[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [error, setError] = useState("");
  const [successMsg, setSuccessMsg] = useState("");

  // Form state. GH #64: a database is now owned by a Site OR a Stack — never both
  // (chk_databases_owner). Stacks are admin-only throughout the panel, so ownerType
  // only ever offers "stack" for admins; a non-admin's request is unaffected.
  const [ownerType, setOwnerType] = useState<"site" | "stack">("site");
  const [siteId, setSiteId] = useState("");
  const [stackId, setStackId] = useState("");
  const [dbName, setDbName] = useState("");
  const [engine, setEngine] = useState("postgres");
  const [submitting, setSubmitting] = useState(false);

  // Delete state
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  // Credentials state
  const [credentialsId, setCredentialsId] = useState<string | null>(null);
  const [credentials, setCredentials] = useState<Credentials | null>(null);
  const [credentialsLoading, setCredentialsLoading] = useState(false);
  const [copied, setCopied] = useState("");
  const [search, setSearch] = useState("");
  const [displayCount, setDisplayCount] = useState(25);

  // Advanced Database Manager & Lethal Confirmation states
  const [manageDb, setManageDb] = useState<{ db: Database; tab?: ManagerTab } | null>(null);
  const [deleteTargetDb, setDeleteTargetDb] = useState<Database | null>(null);

  const { user } = useAuth();
  const isAdmin = user?.role === "admin";

  // Password reset state
  const [resettingPw, setResettingPw] = useState<string | null>(null);
  const [pwResetSuccess, setPwResetSuccess] = useState<string | null>(null);

  const fetchData = async () => {
    try {
      const [dbs, sitesData, stacksData] = await Promise.all([
        api.get<Database[]>("/databases"),
        api.get<Site[]>("/sites"),
        // Stacks are admin-only server-side (require_admin) — skip the call
        // entirely for non-admins rather than surfacing its 403 as a page error.
        isAdmin ? api.get<Stack[]>("/stacks") : Promise.resolve([] as Stack[]),
      ]);
      setDatabases(dbs);
      setSites(sitesData);
      setStacks(stacksData);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load data");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchData();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isAdmin]);

  const handleCreate = async (e: FormEvent) => {
    e.preventDefault();
    setError("");
    setSuccessMsg("");
    setSubmitting(true);
    try {
      await api.post("/databases", {
        site_id: ownerType === "site" ? siteId : undefined,
        stack_id: ownerType === "stack" ? stackId : undefined,
        name: dbName,
        engine,
      });
      setShowForm(false);
      setSuccessMsg(`Database "${dbName}" created`);
      setDbName("");
      fetchData();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create database");
    } finally {
      setSubmitting(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id);
      return;
    }
    setDeletingId(id);
    setSuccessMsg("");
    try {
      await api.delete(`/databases/${id}`);
      setConfirmDeleteId(null);
      setSuccessMsg("Database deleted");
      if (credentialsId === id) {
        setCredentialsId(null);
        setCredentials(null);
      }
      fetchData();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Delete failed");
    } finally {
      setDeletingId(null);
    }
  };

  const toggleCredentials = async (id: string) => {
    if (credentialsId === id) {
      setCredentialsId(null);
      setCredentials(null);
      return;
    }
    setCredentialsId(id);
    setCredentials(null);
    setCredentialsLoading(true);
    try {
      const creds = await api.get<Credentials>(`/databases/${id}/credentials`);
      setCredentials(creds);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load credentials");
      setCredentialsId(null);
    } finally {
      setCredentialsLoading(false);
    }
  };

  const handleResetPassword = async (id: string) => {
    setResettingPw(id);
    setPwResetSuccess(null);
    setError("");
    try {
      const res = await api.post<{ ok: boolean; password: string }>(
        `/databases/${id}/reset-password`
      );
      setPwResetSuccess(res.password);
      // Show the new credentials panel with refreshed data
      setCredentialsId(id);
      setCredentials(null);
      setCredentialsLoading(true);
      try {
        const creds = await api.get<Credentials>(`/databases/${id}/credentials`);
        setCredentials(creds);
      } catch {
        // Credentials will be stale but password is shown in success message
      } finally {
        setCredentialsLoading(false);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Password reset failed");
    } finally {
      setResettingPw(null);
    }
  };

  const copyToClipboard = (text: string, label: string) => {
    navigator.clipboard.writeText(text);
    setCopied(label);
    setTimeout(() => setCopied(""), 2000);
  };

  const getSiteDomain = (siteId: string) =>
    sites.find((s) => s.id === siteId)?.domain || "Unknown";

  const getStackName = (stackId: string) =>
    stacks.find((s) => s.id === stackId)?.name || "Unknown stack";

  const getOwnerLabel = (db: Database) =>
    db.stack_id ? getStackName(db.stack_id) : getSiteDomain(db.site_id ?? "");

  // Advanced Database Manager mode
  if (manageDb) {
    return (
      <div className="p-6 lg:p-8">
        <AdvancedDatabaseManager
          database={manageDb.db}
          initialTab={manageDb.tab}
          onClose={() => {
            setManageDb(null);
            fetchData();
          }}
        />
      </div>
    );
  }

  return (
    <div className="animate-fade-up">
      <div className="page-header">
        <div>
          <h1 className="page-header-title">Databases</h1>
          <p className="page-header-subtitle">Create and manage your databases</p>
        </div>
        <div className="flex items-center gap-2">
          {databases.length >= 2 && (
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search databases..."
              className="px-3 py-1.5 bg-dark-800 border border-dark-600 rounded-lg text-sm text-dark-100 placeholder-dark-400 focus:outline-none focus:border-dark-400"
            />
          )}
          <button
            onClick={() => setShowForm(!showForm)}
            className="px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 transition-colors"
          >
            {showForm ? "Cancel" : "Create Database"}
          </button>
        </div>
      </div>

      <div className="p-6 lg:p-8">

      {error && (
        <div role="alert" className="bg-danger-500/10 text-danger-400 text-sm px-4 py-3 rounded-lg border border-danger-500/20 mb-4">
          {error}
          <button onClick={() => setError("")} className="float-right font-bold" aria-label="Close error">
            &times;
          </button>
        </div>
      )}

      {successMsg && (
        <div className="bg-rust-500/10 text-rust-400 text-sm px-4 py-3 rounded-lg border border-rust-500/20 mb-4">
          {successMsg}
          <button onClick={() => setSuccessMsg("")} className="float-right font-bold" aria-label="Dismiss">
            &times;
          </button>
        </div>
      )}

      {/* Create form */}
      {showForm && (
        <form
          onSubmit={handleCreate}
          className="bg-dark-800 rounded-lg border border-dark-500 p-5 mb-6 space-y-4"
        >
          {/* Owner type toggle — Stacks are admin-only throughout the panel, so this
              only appears once the operator has at least one to pick from. Everyone
              else keeps the plain Site picker unchanged. */}
          {isAdmin && stacks.length > 0 && (
            <div className="flex gap-2" role="radiogroup" aria-label="Database owner type">
              {(["site", "stack"] as const).map((t) => (
                <button
                  key={t}
                  type="button"
                  role="radio"
                  aria-checked={ownerType === t}
                  onClick={() => {
                    setOwnerType(t);
                    setSiteId("");
                    setStackId("");
                    setDbName("");
                  }}
                  className={`px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors ${
                    ownerType === t
                      ? "bg-rust-500/10 text-rust-400 border-rust-500/30"
                      : "text-dark-300 border-dark-600 hover:text-dark-100 hover:border-dark-400"
                  }`}
                >
                  {t === "site" ? "Owned by a Site" : "Owned by a Stack"}
                </button>
              ))}
            </div>
          )}
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            {ownerType === "site" ? (
              <div>
                <label htmlFor="db-site" className="block text-sm font-medium text-dark-100 mb-1">
                  Site
                </label>
                <select
                  id="db-site"
                  value={siteId}
                  onChange={(e) => {
                    const val = e.target.value;
                    setSiteId(val);
                    if (val) {
                      const selectedSite = sites.find((s) => s.id === val);
                      if (selectedSite) {
                        setDbName(selectedSite.domain.replace(/\./g, '_').replace(/-/g, '_'));
                      }
                    }
                  }}
                  className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm bg-dark-800"
                >
                  <option value="">-- Select a site --</option>
                  {sites.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.domain}
                    </option>
                  ))}
                </select>
              </div>
            ) : (
              <div>
                <label htmlFor="db-stack" className="block text-sm font-medium text-dark-100 mb-1">
                  Stack
                </label>
                <select
                  id="db-stack"
                  value={stackId}
                  onChange={(e) => {
                    const val = e.target.value;
                    setStackId(val);
                    if (val) {
                      const selectedStack = stacks.find((s) => s.id === val);
                      if (selectedStack) {
                        setDbName(selectedStack.name.replace(/[^a-zA-Z0-9]/g, '_'));
                      }
                    }
                  }}
                  className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm bg-dark-800"
                >
                  <option value="">-- Select a stack --</option>
                  {stacks.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name}
                    </option>
                  ))}
                </select>
                <p className="text-xs text-dark-300 mt-1">
                  Reachable from the stack's own containers at this database's name — no
                  extra networking to configure.
                </p>
              </div>
            )}
            <div>
              <label htmlFor="db-name" className="block text-sm font-medium text-dark-100 mb-1">
                Database Name
              </label>
              <input
                id="db-name"
                type="text"
                value={dbName}
                onChange={(e) => setDbName(e.target.value)}
                required
                placeholder="my_database"
                pattern="[a-zA-Z0-9][a-zA-Z0-9_]*"
                title="Letters, numbers and underscores. Must start with a letter or a number."
                className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm"
              />
              <p className="text-xs text-dark-300 mt-1">Letters, numbers and underscores — must start with a letter or a number</p>
            </div>
            <div>
              <label htmlFor="db-engine" className="block text-sm font-medium text-dark-100 mb-1">
                Engine
              </label>
              <select
                id="db-engine"
                value={engine}
                onChange={(e) => setEngine(e.target.value)}
                className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm bg-dark-800"
              >
                <option value="postgres">PostgreSQL 16</option>
                <option value="mariadb">MariaDB 11</option>
              </select>
              <p className="text-xs text-dark-300 mt-1">MySQL, MariaDB, or PostgreSQL</p>
            </div>
          </div>
          <div className="flex gap-3">
            <button
              type="submit"
              disabled={submitting}
              className="flex items-center gap-2 px-6 py-2.5 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 disabled:opacity-50 transition-colors"
            >
              {submitting && <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />}
              {submitting ? "Creating..." : "Create Database"}
            </button>
            <button
              type="button"
              onClick={() => setShowForm(false)}
              className="px-4 py-2 text-sm text-dark-300 border border-dark-600 rounded-lg hover:text-dark-100 hover:border-dark-400 transition-colors"
            >
              Cancel
            </button>
          </div>
        </form>
      )}

      {/* Database list */}
      {loading ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 animate-pulse">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="px-5 py-4 border-b border-dark-600 last:border-0">
              <div className="h-5 bg-dark-700 rounded w-48" />
            </div>
          ))}
        </div>
      ) : !showForm && databases.length === 0 ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-12 text-center">
          <svg
            className="w-12 h-12 mx-auto text-dark-300 mb-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={1}
            aria-hidden="true"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M20.25 6.375c0 2.278-3.694 4.125-8.25 4.125S3.75 8.653 3.75 6.375m16.5 0c0-2.278-3.694-4.125-8.25-4.125S3.75 4.097 3.75 6.375m16.5 0v11.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125V6.375m16.5 0v3.75m-16.5-3.75v3.75m16.5 0v3.75C20.25 16.153 16.556 18 12 18s-8.25-1.847-8.25-4.125v-3.75m16.5 0c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125"
            />
          </svg>
          <p className="text-dark-200 font-medium">No databases yet</p>
          <p className="text-dark-300 text-sm mt-2 max-w-md mx-auto">Create MySQL or PostgreSQL databases with automatic Docker containers, user management, and a built-in SQL browser.</p>
          <button onClick={() => setShowForm(true)} className="mt-3 px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 transition-colors">
            Create your first database
          </button>
        </div>
      ) : databases.length > 0 ? (
        <div className="space-y-0">
          <div className="bg-dark-800 rounded-lg border border-dark-500 overflow-x-auto">
            <table className="w-full">
              <thead>
                <tr className="border-b border-dark-500 bg-dark-900">
                  <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-5 py-3">
                    Name
                  </th>
                  <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-5 py-3">
                    Engine
                  </th>
                  <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-5 py-3 hidden md:table-cell">
                    Owner
                  </th>
                  <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-5 py-3 hidden sm:table-cell">
                    Port
                  </th>
                  <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-5 py-3 hidden lg:table-cell">
                    Created
                  </th>
                  <th scope="col" className="text-right text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-5 py-3">
                    Actions
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-dark-600">
                {(() => {
                  const filtered = databases.filter((db) => db.name.toLowerCase().includes(search.toLowerCase()));
                  const displayed = filtered.slice(0, displayCount);
                  return displayed.map((db) => (
                  <tr key={db.id} className="hover:bg-dark-700/30 transition-colors">
                    <td className="px-5 py-4 text-sm font-medium text-dark-50 font-mono">
                      {db.name}
                    </td>
                    <td className="px-5 py-4 text-sm text-dark-200">
                      {engineLabels[db.engine] || db.engine}
                    </td>
                    <td className="px-5 py-4 text-sm text-dark-200 hidden md:table-cell font-mono">
                      {getOwnerLabel(db)}
                    </td>
                    <td className="px-5 py-4 text-sm text-dark-200 font-mono hidden sm:table-cell">
                      {db.port || "\u2014"}
                    </td>
                    <td className="px-5 py-4 text-sm text-dark-200 hidden lg:table-cell">
                      {formatDate(db.created_at)}
                    </td>
                    <td className="px-5 py-4 text-right">
                      <div className="flex items-center justify-end gap-2">
                        <button
                          onClick={() => setManageDb({ db, tab: "browse" })}
                          className="px-2 py-1 rounded text-xs font-medium bg-dark-700 text-dark-200 hover:bg-dark-600 transition-colors"
                        >
                          Browse
                        </button>
                        <button
                          onClick={() => setManageDb({ db, tab: "structure" })}
                          className="px-2 py-1 rounded text-xs font-medium bg-accent-500/10 text-accent-400 hover:bg-accent-500/15 transition-colors"
                        >
                          Structure
                        </button>
                        <button
                          onClick={() => setManageDb({ db, tab: "sql" })}
                          className="px-2 py-1 rounded text-xs font-medium bg-dark-700 text-dark-200 hover:bg-dark-600 transition-colors"
                        >
                          SQL
                        </button>
                        <button
                          onClick={() => setManageDb({ db, tab: "import" })}
                          className="px-2 py-1 rounded text-xs font-medium bg-dark-700 text-dark-200 hover:bg-dark-600 transition-colors"
                          title="Import, upload or export dumps"
                        >
                          Import/Export
                        </button>
                        <button
                          onClick={async () => {
                            try {
                              const cfg = await api.get<{ pitr_enabled: boolean; retention_hours: number }>(`/databases/${db.id}/pitr`);
                              const newEnabled = !cfg.pitr_enabled;
                              await api.put(`/databases/${db.id}/pitr`, {
                                pitr_enabled: newEnabled,
                                retention_hours: cfg.retention_hours || 24,
                              });
                              setError("");
                              // The flag is stored and nothing consumes it: no WAL is
                              // archived, no binlog retained, and the restore endpoint
                              // answers 501. Saying "recovery enabled" promised a
                              // recoverability nothing behind this button can deliver —
                              // the worst place in the product to be optimistic.
                              setSuccessMsg(
                                newEnabled
                                  ? `Retention preference saved for ${db.name}. Point-in-time recovery is not implemented — nothing is being archived, and restore is unavailable. Use scheduled backups.`
                                  : `Retention preference cleared for ${db.name}.`
                              );
                            } catch (e) { setError(e instanceof Error ? e.message : "Failed to save retention preference"); }
                          }}
                          className="px-2 py-1 rounded text-xs font-medium bg-dark-700 text-dark-300 hover:bg-dark-600 transition-colors"
                          title="Store a PITR retention preference. Point-in-time recovery itself is not implemented — nothing is archived and restore returns 501."
                        >
                          PITR
                        </button>
                        <button
                          onClick={() => handleResetPassword(db.id)}
                          disabled={resettingPw === db.id}
                          className="px-2 py-1 rounded text-xs font-medium bg-dark-700 text-dark-300 hover:bg-dark-600 disabled:opacity-50 transition-colors"
                          title="Generate a new database password"
                        >
                          {resettingPw === db.id ? "..." : "Reset PW"}
                        </button>
                        <button
                          onClick={() => toggleCredentials(db.id)}
                          className={`px-2 py-1 rounded text-xs font-medium transition-colors ${
                            credentialsId === db.id
                              ? "bg-rust-500/15 text-rust-400"
                              : "bg-dark-700 text-dark-200 hover:bg-dark-600"
                          }`}
                        >
                          {credentialsId === db.id ? "Hide" : "Connect"}
                        </button>
                        <button
                          onClick={() => setDeleteTargetDb(db)}
                          className="text-xs text-danger-400 hover:text-danger-300"
                        >
                          Delete
                        </button>
                      </div>
                    </td>
                  </tr>
                ));
                })()}
              </tbody>
            </table>
            {(() => {
              const filtered = databases.filter((db) => db.name.toLowerCase().includes(search.toLowerCase()));
              const remaining = filtered.length - displayCount;
              return remaining > 0 ? (
                <button
                  onClick={() => setDisplayCount((c) => c + 25)}
                  className="w-full py-2 text-sm text-dark-300 hover:text-dark-100 border-t border-dark-600 hover:bg-dark-700/30 transition-colors"
                >
                  Show more ({remaining} remaining)
                </button>
              ) : null;
            })()}
          </div>

          {/* Password reset success */}
          {pwResetSuccess && (
            <div className="mt-4 bg-dark-800 rounded-lg border border-accent-500/30 p-4">
              <div className="flex items-center justify-between mb-2">
                <h3 className="text-xs font-medium text-accent-400 uppercase font-mono tracking-widest">
                  Password Reset Successful
                </h3>
                <button
                  onClick={() => setPwResetSuccess(null)}
                  className="text-dark-400 hover:text-dark-200 text-sm"
                  aria-label="Dismiss"
                >
                  &times;
                </button>
              </div>
              <p className="text-xs text-dark-300 mb-2">Save this password now — it will not be shown again after you leave this page.</p>
              <div className="flex items-center gap-2">
                <code className="flex-1 bg-dark-900 border border-dark-500 rounded-lg px-3 py-2 text-sm font-mono text-dark-50">
                  {pwResetSuccess}
                </code>
                <button
                  onClick={() => copyToClipboard(pwResetSuccess, "new_password")}
                  className="shrink-0 px-3 py-2 bg-dark-700 text-dark-200 rounded-lg text-xs hover:bg-dark-600 transition-colors"
                >
                  {copied === "new_password" ? "Copied!" : "Copy"}
                </button>
              </div>
            </div>
          )}

          {/* Credentials panel */}
          {credentialsId && (
            <div className="mt-4 bg-dark-800 rounded-lg border border-dark-500 p-5">
              {credentialsLoading ? (
                <div className="text-center text-dark-300 py-4">Loading credentials...</div>
              ) : credentials ? (
                <div>
                  <h3 className="text-xs font-medium text-dark-300 uppercase font-mono tracking-widest mb-4">
                    Connection Details
                  </h3>

                  {/* Connection string */}
                  <div className="mb-4">
                    <label className="block text-xs font-medium text-dark-200 mb-1">
                      Connection String
                    </label>
                    <div className="flex items-center gap-2">
                      <code className="flex-1 bg-dark-900 border border-dark-500 rounded-lg px-3 py-2 text-xs font-mono text-dark-100 overflow-x-auto">
                        {credentials.connection_string}
                      </code>
                      <button
                        onClick={() =>
                          copyToClipboard(credentials.connection_string, "connection_string")
                        }
                        className="shrink-0 px-3 py-2 bg-dark-700 text-dark-200 rounded-lg text-xs hover:bg-dark-600 transition-colors"
                      >
                        {copied === "connection_string" ? "Copied!" : "Copy"}
                      </button>
                    </div>
                  </div>

                  {/* Individual fields.
                      Two of these carry a `hint`, because two of them are the reason
                      this panel generates support questions. A managed database is
                      published on the host's loopback only and sits on a bridge with
                      container-to-container traffic switched off — so "Host" does not
                      work from inside a container, and "Internal Host" is a container
                      NAME rather than a hostname anything can resolve. Stating that
                      here is the whole fix: the field was inviting a connection
                      attempt that cannot succeed, and then saying nothing when it
                      failed. GitHub #107. */}
                  <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
                    {[
                      { label: "Host", value: credentials.host, key: "host",
                        hint: "From a shell on this server. The port is published on the host's loopback only, so this will not work from inside another container or from off-box." },
                      { label: "Port", value: String(credentials.port), key: "port" },
                      { label: "Database", value: credentials.database, key: "database" },
                      { label: "Username", value: credentials.username, key: "username" },
                      { label: "Password", value: credentials.password, key: "password" },
                      { label: "Internal Host", value: credentials.internal_host, key: "internal_host",
                        hint: "The Docker container's name — for docker exec, docker logs and the CLI. It is not a hostname you can connect to." },
                    ].map((field: { label: string; value: string; key: string; hint?: string }) => (
                      <div key={field.key}>
                        <label className="block text-xs font-medium text-dark-200 mb-1">
                          {field.label}
                        </label>
                        <div className="flex items-center gap-1">
                          <code className="flex-1 bg-dark-900 border border-dark-500 rounded px-2 py-1.5 text-xs font-mono text-dark-100 truncate">
                            {field.key === "password" ? "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022" : field.value}
                          </code>
                          <button
                            onClick={() => copyToClipboard(field.value, field.key)}
                            className="shrink-0 p-1.5 text-dark-300 hover:text-dark-200"
                            title={`Copy ${field.label}`}
                          >
                            {copied === field.key ? (
                              <svg className="w-3.5 h-3.5 text-rust-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                                <path strokeLinecap="round" strokeLinejoin="round" d="M4.5 12.75l6 6 9-13.5" />
                              </svg>
                            ) : (
                              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                                <path strokeLinecap="round" strokeLinejoin="round" d="M15.666 3.888A2.25 2.25 0 0 0 13.5 2.25h-3c-1.03 0-1.9.693-2.166 1.638m7.332 0c.055.194.084.4.084.612v0a.75.75 0 0 1-.75.75H9.75a.75.75 0 0 1-.75-.75v0c0-.212.03-.418.084-.612m7.332 0c.646.049 1.288.11 1.927.184 1.1.128 1.907 1.077 1.907 2.185V19.5a2.25 2.25 0 0 1-2.25 2.25H6.75A2.25 2.25 0 0 1 4.5 19.5V6.257c0-1.108.806-2.057 1.907-2.185a48.208 48.208 0 0 1 1.927-.184" />
                              </svg>
                            )}
                          </button>
                        </div>
                        {field.hint && (
                          <p className="text-xs text-dark-400 mt-1 leading-snug">{field.hint}</p>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
            </div>
          )}
        </div>
      ) : null}
      {/* Database Deletion Consequence Dialog */}
      {deleteTargetDb && (
        <ConfirmDialog
          title={`Delete Database: ${deleteTargetDb.name}`}
          label={`Are you sure you want to permanently delete database "${deleteTargetDb.name}"?`}
          consequences={[
            `Permanently drops database "${deleteTargetDb.name}" and removes its underlying container`,
            "All tables, schemas, stored records, and credentials will be irreversibly erased",
            "Any websites, APIs, or Docker apps connected to this database will immediately experience connection errors",
            "Saved backups in /var/backups will no longer be linked to an active database container"
          ]}
          confirmLabel="Permanently Delete Database"
          tone="danger"
          onConfirm={() => {
            const id = deleteTargetDb.id;
            setDeleteTargetDb(null);
            handleDelete(id);
          }}
          onCancel={() => setDeleteTargetDb(null)}
        />
      )}
      </div>
    </div>
  );
}
