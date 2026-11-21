import { useState, useEffect, useCallback, useRef } from "react";
import { useParams, Link } from "react-router-dom";
import { api } from "../api";
import { UPLOAD_MAX_FILE_BYTES, uploadTooLargeMessage } from "../constants";

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: string;
  permissions?: string;
  owner?: string;
}

interface DirItem {
  name: string;
  path: string;
  has_subdirs: boolean;
}

interface TreeNode {
  name: string;
  path: string;
  has_subdirs: boolean;
  is_expanded?: boolean;
  children?: TreeNode[];
  loading?: boolean;
}

interface Site {
  id: string;
  domain: string;
}

function formatSize(bytes: number): string {
  if (bytes === 0) return "—";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

function formatDate(iso: string): string {
  if (!iso) return "—";
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function isArchive(name: string): boolean {
  const lower = name.toLowerCase();
  return (
    lower.endsWith(".zip") ||
    lower.endsWith(".tar.gz") ||
    lower.endsWith(".tgz") ||
    lower.endsWith(".tar") ||
    lower.endsWith(".tar.bz2")
  );
}

function isEditable(name: string): boolean {
  const lower = name.toLowerCase();
  const editableExts = [
    ".txt", ".md", ".html", ".htm", ".css", ".scss", ".js", ".jsx", ".ts", ".tsx",
    ".json", ".xml", ".yaml", ".yml", ".php", ".py", ".sh", ".bash", ".env",
    ".conf", ".config", ".htaccess", ".sql", ".ini", ".log", ".svg", ".lock",
    ".toml", ".rs", ".go", ".c", ".cpp", ".h"
  ];
  return editableExts.some((ext) => lower.endsWith(ext)) || !name.includes(".");
}

export default function Files() {
  const { id } = useParams<{ id: string }>();
  const [site, setSite] = useState<Site | null>(null);
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [currentPath, setCurrentPath] = useState(".");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [successMessage, setSuccessMessage] = useState("");

  // Directory Tree
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [treeNodes, setTreeNodes] = useState<TreeNode[]>([]);
  const [treeLoading, setTreeLoading] = useState(false);

  // Selection
  const [selected, setSelected] = useState<Set<string>>(new Set());

  // Editor
  const [editing, setEditing] = useState<string | null>(null);
  const [editorContent, setEditorContent] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [editorFullscreen, setEditorFullscreen] = useState(false);
  const editorRef = useRef<HTMLTextAreaElement>(null);

  // Create Modal
  const [showCreate, setShowCreate] = useState(false);
  const [createName, setCreateName] = useState("");
  const [createType, setCreateType] = useState<"file" | "dir">("file");
  const [creating, setCreating] = useState(false);

  // Rename Modal
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameTo, setRenameTo] = useState("");
  const [renamingLoading, setRenamingLoading] = useState(false);

  // Delete Modal
  const [deleteTargets, setDeleteTargets] = useState<string[]>([]);
  const [deleting, setDeleting] = useState(false);

  // Extract Modal
  const [extractArchiveTarget, setExtractArchiveTarget] = useState<string | null>(null);
  const [extractDestination, setExtractDestination] = useState<string>(".");
  const [extracting, setExtracting] = useState(false);

  // Compress Modal
  const [showCompress, setShowCompress] = useState(false);
  const [compressName, setCompressName] = useState("archive.zip");
  const [compressFormat, setCompressFormat] = useState<"zip" | "tar.gz">("zip");
  const [compressing, setCompressing] = useState(false);

  // Copy / Move Modal
  const [copyMoveAction, setCopyMoveAction] = useState<"copy" | "move" | null>(null);
  const [copyMoveDestination, setCopyMoveDestination] = useState<string>(".");
  const [copyMoveLoading, setCopyMoveLoading] = useState(false);

  // Chmod Modal
  const [chmodTarget, setChmodTarget] = useState<string | null>(null);
  const [chmodMode, setChmodMode] = useState<string>("0755");
  const [chmodRecursive, setChmodRecursive] = useState(false);
  const [chmodLoading, setChmodLoading] = useState(false);

  // Chown Modal
  const [chownTarget, setChownTarget] = useState<string | null>(null);
  const [chownOwner, setChownOwner] = useState<string>("www-data:www-data");
  const [chownRecursive, setChownRecursive] = useState(true);
  const [chownLoading, setChownLoading] = useState(false);

  // Upload
  const [uploading, setUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<{
    fileIndex: number;
    totalFiles: number;
    fileName: string;
    fileSize: number;
    loadedBytes: number;
    percent: number;
    phase: "reading" | "uploading" | "processing";
  } | null>(null);
  const [uploadMessage, setUploadMessage] = useState("");

  const showSuccess = (msg: string) => {
    setSuccessMessage(msg);
    setTimeout(() => setSuccessMessage(""), 4000);
  };

  useEffect(() => {
    api.get<Site>(`/sites/${id}`).then(setSite).catch(() => {});
  }, [id]);

  // Load Tree Root
  const loadTreeRoot = useCallback(async () => {
    setTreeLoading(true);
    try {
      const data = await api.get<DirItem[]>(`/sites/${id}/files/tree?path=.`);
      setTreeNodes(
        data.map((item) => ({
          name: item.name,
          path: item.path,
          has_subdirs: item.has_subdirs,
          is_expanded: false,
          children: undefined,
        }))
      );
    } catch {
      // Tree is optional helper
    } finally {
      setTreeLoading(false);
    }
  }, [id]);

  useEffect(() => {
    loadTreeRoot();
  }, [loadTreeRoot]);

  // Expand / Collapse Tree Node
  const toggleTreeNode = async (node: TreeNode) => {
    const updateNode = (
      list: TreeNode[],
      targetPath: string,
      transform: (n: TreeNode) => TreeNode
    ): TreeNode[] => {
      return list.map((item) => {
        if (item.path === targetPath) {
          return transform(item);
        }
        if (item.children) {
          return {
            ...item,
            children: updateNode(item.children, targetPath, transform),
          };
        }
        return item;
      });
    };

    if (node.is_expanded) {
      // Collapse
      setTreeNodes((prev) =>
        updateNode(prev, node.path, (n) => ({ ...n, is_expanded: false }))
      );
      return;
    }

    // Expand
    if (node.children) {
      setTreeNodes((prev) =>
        updateNode(prev, node.path, (n) => ({ ...n, is_expanded: true }))
      );
      return;
    }

    // Fetch children
    setTreeNodes((prev) =>
      updateNode(prev, node.path, (n) => ({ ...n, loading: true }))
    );
    try {
      const data = await api.get<DirItem[]>(
        `/sites/${id}/files/tree?path=${encodeURIComponent(node.path)}`
      );
      const children: TreeNode[] = data.map((c) => ({
        name: c.name,
        path: c.path,
        has_subdirs: c.has_subdirs,
        is_expanded: false,
      }));
      setTreeNodes((prev) =>
        updateNode(prev, node.path, (n) => ({
          ...n,
          loading: false,
          is_expanded: true,
          children,
        }))
      );
    } catch {
      setTreeNodes((prev) =>
        updateNode(prev, node.path, (n) => ({
          ...n,
          loading: false,
          is_expanded: true,
          children: [],
        }))
      );
    }
  };

  const loadDir = useCallback(
    async (path: string) => {
      setLoading(true);
      setError("");
      setSelected(new Set());
      try {
        const data = await api.get<FileEntry[]>(
          `/sites/${id}/files?path=${encodeURIComponent(path)}`
        );
        setEntries(data);
        setCurrentPath(path);
      } catch (e) {
        setError(e instanceof Error ? e.message : "Failed to load directory");
      } finally {
        setLoading(false);
      }
    },
    [id]
  );

  useEffect(() => {
    loadDir(".");
  }, [loadDir]);

  const navigateTo = (name: string) => {
    const newPath = currentPath === "." ? name : `${currentPath}/${name}`;
    loadDir(newPath);
    setEditing(null);
  };

  const goUp = () => {
    if (currentPath === ".") return;
    const parts = currentPath.split("/");
    parts.pop();
    loadDir(parts.length === 0 ? "." : parts.join("/"));
    setEditing(null);
  };

  const openFile = async (name: string) => {
    const filePath = currentPath === "." ? name : `${currentPath}/${name}`;
    try {
      const data = await api.get<{ content: string }>(
        `/sites/${id}/files/read?path=${encodeURIComponent(filePath)}`
      );
      setEditorContent(data.content);
      setEditing(filePath);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to read file");
    }
  };

  const saveFile = async () => {
    if (!editing) return;
    setSaving(true);
    setSaveSuccess(false);
    try {
      await api.put(`/sites/${id}/files/write`, {
        path: editing,
        content: editorContent,
      });
      setSaveSuccess(true);
      setTimeout(() => setSaveSuccess(false), 2000);
      loadDir(currentPath);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to save file");
    } finally {
      setSaving(false);
    }
  };

  const handleCreate = async () => {
    if (!createName.trim()) return;
    setCreating(true);
    const path =
      currentPath === "." ? createName.trim() : `${currentPath}/${createName.trim()}`;
    try {
      await api.post(
        `/sites/${id}/files/create?path=${encodeURIComponent(path)}&type=${createType}`
      );
      setShowCreate(false);
      setCreateName("");
      showSuccess(`Created ${createType === "dir" ? "folder" : "file"} "${createName.trim()}"`);
      loadDir(currentPath);
      loadTreeRoot();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to create");
    } finally {
      setCreating(false);
    }
  };

  const handleRename = async () => {
    if (!renaming || !renameTo.trim()) return;
    setRenamingLoading(true);
    const fromPath =
      currentPath === "." ? renaming : `${currentPath}/${renaming}`;
    const toPath =
      currentPath === "." ? renameTo.trim() : `${currentPath}/${renameTo.trim()}`;
    try {
      await api.post(`/sites/${id}/files/rename`, {
        from: fromPath,
        to: toPath,
      });
      setRenaming(null);
      setRenameTo("");
      showSuccess(`Renamed to "${renameTo.trim()}"`);
      loadDir(currentPath);
      loadTreeRoot();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to rename");
    } finally {
      setRenamingLoading(false);
    }
  };

  const handleDeleteConfirmed = async () => {
    if (deleteTargets.length === 0) return;
    setDeleting(true);
    try {
      if (deleteTargets.length === 1) {
        const path =
          currentPath === "." ? deleteTargets[0] : `${currentPath}/${deleteTargets[0]}`;
        await api.delete(`/sites/${id}/files?path=${encodeURIComponent(path)}`);
      } else {
        const paths = deleteTargets.map((name) =>
          currentPath === "." ? name : `${currentPath}/${name}`
        );
        await api.post(`/sites/${id}/files/delete-batch`, { paths });
      }
      showSuccess(`Deleted ${deleteTargets.length} item(s)`);
      setDeleteTargets([]);
      setSelected(new Set());
      loadDir(currentPath);
      loadTreeRoot();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to delete");
    } finally {
      setDeleting(false);
    }
  };

  // Extract
  const handleExtract = async () => {
    if (!extractArchiveTarget) return;
    setExtracting(true);
    try {
      const archivePath =
        currentPath === "."
          ? extractArchiveTarget
          : `${currentPath}/${extractArchiveTarget}`;
      await api.post(`/sites/${id}/files/extract`, {
        archive: archivePath,
        destination: extractDestination.trim() || currentPath,
      });
      showSuccess(`Successfully extracted "${extractArchiveTarget}"`);
      setExtractArchiveTarget(null);
      loadDir(currentPath);
      loadTreeRoot();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to extract archive");
    } finally {
      setExtracting(false);
    }
  };

  // Compress
  const handleCompress = async () => {
    if (selected.size === 0 || !compressName.trim()) return;
    setCompressing(true);
    try {
      const sources = Array.from(selected).map((name) =>
        currentPath === "." ? name : `${currentPath}/${name}`
      );
      const destination =
        currentPath === "."
          ? compressName.trim()
          : `${currentPath}/${compressName.trim()}`;

      await api.post(`/sites/${id}/files/compress`, {
        sources,
        destination,
        format: compressFormat,
      });
      showSuccess(`Created archive "${compressName.trim()}"`);
      setShowCompress(false);
      setSelected(new Set());
      loadDir(currentPath);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to compress files");
    } finally {
      setCompressing(false);
    }
  };

  // Copy / Move
  const handleCopyMove = async () => {
    if (!copyMoveAction || selected.size === 0) return;
    setCopyMoveLoading(true);
    try {
      const sources = Array.from(selected).map((name) =>
        currentPath === "." ? name : `${currentPath}/${name}`
      );
      const destinationDir = copyMoveDestination.trim() || ".";

      if (copyMoveAction === "copy") {
        await api.post(`/sites/${id}/files/copy`, {
          sources,
          destination_dir: destinationDir,
        });
        showSuccess(`Copied ${sources.length} item(s) to ${destinationDir}`);
      } else {
        await api.post(`/sites/${id}/files/move`, {
          sources,
          destination_dir: destinationDir,
        });
        showSuccess(`Moved ${sources.length} item(s) to ${destinationDir}`);
      }
      setCopyMoveAction(null);
      setSelected(new Set());
      loadDir(currentPath);
      loadTreeRoot();
    } catch (e) {
      setError(e instanceof Error ? e.message : `Failed to ${copyMoveAction}`);
    } finally {
      setCopyMoveLoading(false);
    }
  };

  // Chmod
  const handleChmod = async () => {
    if (!chmodTarget || !chmodMode.trim()) return;
    setChmodLoading(true);
    try {
      const targetPath =
        currentPath === "." ? chmodTarget : `${currentPath}/${chmodTarget}`;
      await api.post(`/sites/${id}/files/chmod`, {
        path: targetPath,
        mode: chmodMode.trim(),
        recursive: chmodRecursive,
      });
      showSuccess(`Permissions updated to ${chmodMode.trim()}`);
      setChmodTarget(null);
      loadDir(currentPath);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to update permissions");
    } finally {
      setChmodLoading(false);
    }
  };

  // Chown
  const handleChown = async () => {
    if (!chownTarget || !chownOwner.trim()) return;
    setChownLoading(true);
    try {
      const targetPath =
        currentPath === "." ? chownTarget : `${currentPath}/${chownTarget}`;
      await api.post(`/sites/${id}/files/chown`, {
        path: targetPath,
        owner: chownOwner.trim(),
        recursive: chownRecursive,
      });
      showSuccess(`Ownership updated to ${chownOwner.trim()}`);
      setChownTarget(null);
      loadDir(currentPath);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to update ownership");
    } finally {
      setChownLoading(false);
    }
  };

  // Selection toggle
  const toggleSelect = (name: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const toggleSelectAll = () => {
    if (selected.size === entries.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(entries.map((e) => e.name)));
    }
  };

  const breadcrumbs = currentPath === "." ? [] : currentPath.split("/");

  // Recursive Tree Node Renderer
  const renderTreeItem = (node: TreeNode, depth: number = 0) => {
    const isCurrent = currentPath === node.path;
    return (
      <div key={node.path} className="select-none">
        <div
          className={`flex items-center gap-1.5 py-1 px-2 rounded-md text-xs font-mono transition-colors cursor-pointer ${
            isCurrent
              ? "bg-rust-500/20 text-rust-300 font-semibold"
              : "text-dark-200 hover:bg-dark-700/50 hover:text-dark-50"
          }`}
          style={{ paddingLeft: `${depth * 14 + 8}px` }}
        >
          {node.has_subdirs ? (
            <button
              onClick={(e) => {
                e.stopPropagation();
                toggleTreeNode(node);
              }}
              className="p-0.5 text-dark-400 hover:text-dark-200 focus:outline-none"
            >
              {node.loading ? (
                <div className="w-3 h-3 border border-dark-400 border-t-rust-400 rounded-full animate-spin" />
              ) : (
                <svg
                  className={`w-3 h-3 transition-transform ${
                    node.is_expanded ? "rotate-90 text-rust-400" : ""
                  }`}
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2.5}
                >
                  <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
                </svg>
              )}
            </button>
          ) : (
            <span className="w-4" />
          )}

          <div
            onClick={() => loadDir(node.path)}
            className="flex items-center gap-1.5 flex-1 truncate"
          >
            <svg
              className={`w-3.5 h-3.5 shrink-0 ${
                isCurrent || node.is_expanded ? "text-warn-400" : "text-dark-400"
              }`}
              fill="currentColor"
              viewBox="0 0 20 20"
            >
              <path d="M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
            </svg>
            <span className="truncate">{node.name}</span>
          </div>
        </div>

        {node.is_expanded && node.children && (
          <div className="flex flex-col">
            {node.children.length === 0 ? (
              <div
                className="text-[11px] text-dark-400 italic py-0.5"
                style={{ paddingLeft: `${(depth + 1) * 14 + 18}px` }}
              >
                (empty)
              </div>
            ) : (
              node.children.map((child) => renderTreeItem(child, depth + 1))
            )}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="p-6 lg:p-8 flex flex-col min-h-screen">
      {/* Top Header / Breadcrumbs */}
      <div className="mb-4">
        <div className="flex items-center gap-2 text-sm text-dark-300 mb-2">
          <Link to={`/sites/${id}`} className="hover:text-dark-100 font-mono">
            {site?.domain || "Site"}
          </Link>
          <span>/</span>
          <span className="text-dark-50 font-medium">Files</span>
        </div>
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div>
            <h1 className="text-sm font-medium text-dark-300 uppercase font-mono tracking-widest">
              File Manager
            </h1>
            <p className="text-xs text-dark-400 font-mono mt-0.5">
              Root: /var/www/{site?.domain}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <button
              onClick={() => setSidebarOpen((v) => !v)}
              className={`px-3 py-1.5 rounded-lg text-xs font-mono font-medium border transition-colors flex items-center gap-1.5 ${
                sidebarOpen
                  ? "bg-dark-700 text-dark-200 border-dark-600"
                  : "bg-dark-800 text-dark-400 border-dark-600 hover:text-dark-200"
              }`}
              title="Toggle Directory Tree"
            >
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25H12" />
              </svg>
              Directory Tree
            </button>

            <button
              onClick={() => {
                setCreateType("dir");
                setShowCreate(true);
              }}
              className="px-3 py-1.5 bg-dark-700 text-dark-100 hover:bg-dark-600 rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5"
            >
              <svg className="w-3.5 h-3.5 text-warn-500" fill="currentColor" viewBox="0 0 20 20">
                <path d="M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
              </svg>
              New Folder
            </button>

            <button
              onClick={() => {
                setCreateType("file");
                setShowCreate(true);
              }}
              className="px-3 py-1.5 bg-dark-700 text-dark-100 hover:bg-dark-600 rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5"
            >
              <svg className="w-3.5 h-3.5 text-accent-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
              </svg>
              New File
            </button>

            <label
              className={`px-4 py-1.5 bg-rust-500 text-white rounded-lg text-xs font-medium hover:bg-rust-600 cursor-pointer transition-colors flex items-center gap-1.5 ${
                uploading ? "opacity-50 pointer-events-none" : ""
              }`}
            >
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
              </svg>
              {uploading
                ? uploadProgress
                  ? `Uploading ${uploadProgress.percent}%`
                  : "Uploading..."
                : "Upload"}
              <input
                type="file"
                className="hidden"
                multiple
                disabled={uploading}
                onChange={async (e) => {
                  const files = e.target.files;
                  if (!files || files.length === 0) return;
                  const fileList = Array.from(files);
                  setUploading(true);
                  setUploadMessage("");
                  let successCount = 0;
                  let failedCount = 0;
                  let firstError = "";

                  for (let i = 0; i < fileList.length; i++) {
                    const file = fileList[i];
                    if (file.size > UPLOAD_MAX_FILE_BYTES) {
                      failedCount++;
                      if (!firstError) firstError = uploadTooLargeMessage(file.name, file.size);
                      continue;
                    }
                    setUploadProgress({
                      fileIndex: i + 1,
                      totalFiles: fileList.length,
                      fileName: file.name,
                      fileSize: file.size,
                      loadedBytes: 0,
                      percent: 0,
                      phase: "reading",
                    });
                    try {
                      const base64 = await new Promise<string>((resolve, reject) => {
                        const reader = new FileReader();
                        reader.onprogress = (event) => {
                          if (event.lengthComputable) {
                            const pct = Math.min(100, Math.round((event.loaded / event.total) * 100));
                            setUploadProgress({
                              fileIndex: i + 1,
                              totalFiles: fileList.length,
                              fileName: file.name,
                              fileSize: file.size,
                              loadedBytes: event.loaded,
                              percent: pct,
                              phase: "reading",
                            });
                          }
                        };
                        reader.onload = () => resolve((reader.result as string).split(",")[1]);
                        reader.onerror = () => reject(new Error(`Failed to read ${file.name}`));
                        reader.readAsDataURL(file);
                      });

                      setUploadProgress({
                        fileIndex: i + 1,
                        totalFiles: fileList.length,
                        fileName: file.name,
                        fileSize: file.size,
                        loadedBytes: 0,
                        percent: 0,
                        phase: "uploading",
                      });

                      await api.upload(
                        `/sites/${id}/files/upload`,
                        {
                          path: currentPath,
                          filename: file.name,
                          content: base64,
                        },
                        (loaded, total) => {
                          const pct = Math.min(100, Math.round((loaded / total) * 100));
                          const estimatedLoaded = Math.min(
                            file.size,
                            Math.round((loaded / total) * file.size)
                          );
                          setUploadProgress({
                            fileIndex: i + 1,
                            totalFiles: fileList.length,
                            fileName: file.name,
                            fileSize: file.size,
                            loadedBytes: estimatedLoaded,
                            percent: pct,
                            phase: pct >= 100 ? "processing" : "uploading",
                          });
                        }
                      );
                      successCount++;
                    } catch (err) {
                      failedCount++;
                      if (!firstError) {
                        firstError = err instanceof Error ? err.message : `Could not upload ${file.name}`;
                      }
                    }
                  }
                  setUploadProgress(null);
                  setUploading(false);
                  if (failedCount > 0) {
                    setUploadMessage(
                      successCount > 0
                        ? `${successCount} uploaded, ${failedCount} failed — ${firstError}`
                        : firstError
                    );
                  } else {
                    setUploadMessage(`${successCount} file(s) uploaded successfully`);
                  }
                  loadDir(currentPath);
                  e.target.value = "";
                  if (failedCount === 0) setTimeout(() => setUploadMessage(""), 4000);
                }}
              />
            </label>
          </div>
        </div>
      </div>

      {/* Path Breadcrumbs Bar */}
      <div className="bg-dark-800 rounded-lg border border-dark-600 px-4 py-2 mb-4 flex items-center justify-between gap-2 text-xs font-mono">
        <div className="flex items-center gap-1 overflow-x-auto py-0.5">
          <button
            onClick={() => loadDir(".")}
            className="text-rust-400 hover:text-rust-300 font-bold px-1 py-0.5 rounded hover:bg-dark-700"
          >
            /
          </button>
          {breadcrumbs.map((part, i) => (
            <span key={i} className="flex items-center gap-1 shrink-0">
              <span className="text-dark-500">/</span>
              {i < breadcrumbs.length - 1 ? (
                <button
                  onClick={() => loadDir(breadcrumbs.slice(0, i + 1).join("/"))}
                  className="text-rust-400 hover:text-rust-300 px-1 py-0.5 rounded hover:bg-dark-700"
                >
                  {part}
                </button>
              ) : (
                <span className="text-dark-100 font-medium px-1 py-0.5">{part}</span>
              )}
            </span>
          ))}
        </div>
        <div className="flex items-center gap-3 shrink-0">
          <span className="text-dark-400 text-[11px]">
            {entries.length} items
          </span>
          <button
            onClick={() => loadDir(currentPath)}
            className="p-1 text-dark-300 hover:text-dark-100 transition-colors"
            title="Refresh current folder"
          >
            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99" />
            </svg>
          </button>
        </div>
      </div>

      {/* Progress & Alert Banners */}
      {uploadProgress && (
        <div className="mb-4 bg-dark-800 border border-dark-600 rounded-lg p-4 shadow-sm animate-fade-in">
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2 overflow-hidden">
              <div className="w-4 h-4 border-2 border-dark-500 border-t-rust-500 rounded-full animate-spin shrink-0" />
              <span className="text-sm font-medium text-dark-100 truncate">
                {uploadProgress.totalFiles > 1 ? `[${uploadProgress.fileIndex}/${uploadProgress.totalFiles}] ` : ""}
                {uploadProgress.fileName}
              </span>
              <span className="text-xs text-dark-400 font-mono shrink-0">
                ({formatSize(uploadProgress.fileSize)})
              </span>
            </div>
            <span className="text-xs font-mono text-rust-400 font-medium">
              {uploadProgress.phase === "reading"
                ? `Preparing... ${uploadProgress.percent}%`
                : uploadProgress.phase === "processing"
                ? "Writing to disk..."
                : `${uploadProgress.percent}%`}
            </span>
          </div>
          <div className="w-full h-2 bg-dark-700 rounded-full overflow-hidden">
            <div
              className={`h-full bg-rust-500 rounded-full transition-all duration-150 ${
                uploadProgress.phase === "processing" ? "animate-pulse" : ""
              }`}
              style={{ width: `${uploadProgress.percent}%` }}
            />
          </div>
        </div>
      )}

      {error && (
        <div className="mb-4 px-4 py-3 bg-danger-500/10 text-danger-400 rounded-lg border border-danger-500/20 text-sm flex items-center justify-between">
          <span>{error}</span>
          <button onClick={() => setError("")} className="text-xs underline hover:text-danger-300">
            Dismiss
          </button>
        </div>
      )}

      {successMessage && (
        <div className="mb-4 px-4 py-3 bg-emerald-500/10 text-emerald-400 rounded-lg border border-emerald-500/20 text-sm">
          {successMessage}
        </div>
      )}

      {uploadMessage && (
        <div
          className={`mb-4 px-4 py-3 rounded-lg text-sm border ${
            uploadMessage.includes("failed")
              ? "bg-danger-500/10 text-danger-400 border-danger-500/20"
              : "bg-rust-500/10 text-rust-400 border-rust-500/20"
          }`}
        >
          {uploadMessage}
        </div>
      )}

      {/* Multi-Selection Batch Actions Floating Bar */}
      {selected.size > 0 && (
        <div className="mb-4 bg-dark-900 border border-rust-500/40 rounded-lg p-3 flex flex-wrap items-center justify-between gap-3 shadow-lg animate-fade-in">
          <div className="flex items-center gap-3">
            <span className="text-xs font-mono bg-rust-500/20 text-rust-300 px-2.5 py-1 rounded-md font-medium">
              {selected.size} selected
            </span>
            <button
              onClick={() => setSelected(new Set())}
              className="text-xs text-dark-300 hover:text-dark-100 underline"
            >
              Clear
            </button>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {selected.size === 1 && isArchive(Array.from(selected)[0]) && (
              <button
                onClick={() => {
                  setExtractArchiveTarget(Array.from(selected)[0]);
                  setExtractDestination(currentPath);
                }}
                className="px-3 py-1 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors flex items-center gap-1.5"
              >
                <svg className="w-3.5 h-3.5 text-warn-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
                </svg>
                Extract
              </button>
            )}

            <button
              onClick={() => {
                setCompressName("archive.zip");
                setShowCompress(true);
              }}
              className="px-3 py-1 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors flex items-center gap-1.5"
            >
              <svg className="w-3.5 h-3.5 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M20.25 7.5l-.625 10.632a2.25 2.25 0 01-2.247 2.118H6.622a2.25 2.25 0 01-2.247-2.118L3.75 7.5M10 11.25h4M3.375 7.5h17.25c.621 0 1.125-.504 1.125-1.125v-1.5c0-.621-.504-1.125-1.125-1.125H3.375c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125z" />
              </svg>
              Compress
            </button>

            <button
              onClick={() => {
                setCopyMoveAction("copy");
                setCopyMoveDestination(currentPath);
              }}
              className="px-3 py-1 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
            >
              Copy to...
            </button>

            <button
              onClick={() => {
                setCopyMoveAction("move");
                setCopyMoveDestination(currentPath);
              }}
              className="px-3 py-1 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
            >
              Move to...
            </button>

            {selected.size === 1 && (
              <button
                onClick={() => {
                  const target = Array.from(selected)[0];
                  const entry = entries.find((e) => e.name === target);
                  setChmodTarget(target);
                  setChmodMode(entry?.permissions || "0755");
                  setChmodRecursive(entry?.is_dir || false);
                }}
                className="px-3 py-1 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
              >
                Chmod
              </button>
            )}

            <button
              onClick={() => setDeleteTargets(Array.from(selected))}
              className="px-3 py-1 bg-danger-500/20 hover:bg-danger-500/30 text-danger-400 rounded text-xs font-medium transition-colors"
            >
              Delete ({selected.size})
            </button>
          </div>
        </div>
      )}

      {/* Main Workspace Layout */}
      <div className="flex gap-4 flex-1 items-start">
        {/* Left Directory Tree Sidebar */}
        {sidebarOpen && (
          <div className="w-64 shrink-0 bg-dark-800 rounded-lg border border-dark-600 p-3 flex flex-col h-[650px] overflow-hidden">
            <div className="flex items-center justify-between pb-2 mb-2 border-b border-dark-600">
              <span className="text-[11px] font-mono uppercase tracking-wider text-dark-400 font-semibold">
                Directories
              </span>
              <button
                onClick={loadTreeRoot}
                disabled={treeLoading}
                className="p-1 text-dark-400 hover:text-dark-200 transition-colors"
                title="Refresh Directory Tree"
              >
                <svg className={`w-3.5 h-3.5 ${treeLoading ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99" />
                </svg>
              </button>
            </div>

            <div className="flex-1 overflow-y-auto space-y-1 pr-1 custom-scrollbar">
              {/* Site Root Node */}
              <div
                onClick={() => loadDir(".")}
                className={`flex items-center gap-2 py-1 px-2 rounded-md text-xs font-mono cursor-pointer transition-colors ${
                  currentPath === "."
                    ? "bg-rust-500/20 text-rust-300 font-semibold"
                    : "text-dark-200 hover:bg-dark-700/50 hover:text-dark-50"
                }`}
              >
                <svg className="w-4 h-4 text-warn-400 shrink-0" fill="currentColor" viewBox="0 0 20 20">
                  <path d="M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
                </svg>
                <span className="truncate">{site?.domain || "Root"}</span>
              </div>

              {/* Subdirectories */}
              {treeNodes.map((node) => renderTreeItem(node, 1))}

              {treeNodes.length === 0 && !treeLoading && (
                <div className="text-[11px] text-dark-400 py-4 text-center">
                  No subdirectories
                </div>
              )}
            </div>
          </div>
        )}

        {/* Center: File Listing Table */}
        <div
          className={`bg-dark-800 rounded-lg border border-dark-600 overflow-hidden flex flex-col ${
            editing && !editorFullscreen ? "w-1/2" : "flex-1"
          } ${editing && editorFullscreen ? "hidden" : ""}`}
        >
          {loading ? (
            <div className="flex items-center justify-center py-20">
              <div className="w-6 h-6 border-2 border-dark-500 border-t-rust-500 rounded-full animate-spin" />
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-left text-xs">
                <thead>
                  <tr className="bg-dark-900 border-b border-dark-600 text-dark-300 font-mono uppercase tracking-wider text-[11px]">
                    <th scope="col" className="p-3 w-8">
                      <input
                        type="checkbox"
                        checked={entries.length > 0 && selected.size === entries.length}
                        onChange={toggleSelectAll}
                        className="rounded border-dark-500 bg-dark-700 text-rust-500 focus:ring-0 cursor-pointer"
                      />
                    </th>
                    <th scope="col" className="px-3 py-3 font-medium">Name</th>
                    <th scope="col" className="px-3 py-3 font-medium w-24">Size</th>
                    <th scope="col" className="px-3 py-3 font-medium w-24">Permissions</th>
                    <th scope="col" className="px-3 py-3 font-medium w-28">Owner</th>
                    <th scope="col" className="px-3 py-3 font-medium w-32">Modified</th>
                    <th scope="col" className="px-3 py-3 font-medium text-right w-36">Actions</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-dark-700 font-mono">
                  {currentPath !== "." && (
                    <tr
                      className="hover:bg-dark-700/40 transition-colors cursor-pointer"
                      onClick={goUp}
                    >
                      <td className="p-3" />
                      <td colSpan={6} className="px-3 py-2.5 text-dark-200 flex items-center gap-2 font-medium">
                        <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M15 19l-7-7 7-7" />
                        </svg>
                        .. (Up one level)
                      </td>
                    </tr>
                  )}

                  {entries.length === 0 && currentPath === "." && (
                    <tr>
                      <td colSpan={7} className="px-4 py-16 text-center text-dark-400 text-xs">
                        This directory is empty.
                      </td>
                    </tr>
                  )}

                  {entries.map((entry) => {
                    const isArch = isArchive(entry.name);
                    const isEdit = isEditable(entry.name);
                    const isSelected = selected.has(entry.name);

                    return (
                      <tr
                        key={entry.name}
                        className={`hover:bg-dark-700/40 transition-colors group ${
                          isSelected ? "bg-rust-500/10" : ""
                        }`}
                      >
                        {/* Checkbox */}
                        <td className="p-3" onClick={(e) => e.stopPropagation()}>
                          <input
                            type="checkbox"
                            checked={isSelected}
                            onChange={() => toggleSelect(entry.name)}
                            className="rounded border-dark-500 bg-dark-700 text-rust-500 focus:ring-0 cursor-pointer"
                          />
                        </td>

                        {/* Name */}
                        <td className="px-3 py-2.5">
                          <button
                            onClick={() =>
                              entry.is_dir
                                ? navigateTo(entry.name)
                                : isEdit
                                ? openFile(entry.name)
                                : null
                            }
                            className={`flex items-center gap-2 hover:underline text-left ${
                              entry.is_dir
                                ? "text-dark-50 font-semibold"
                                : isArch
                                ? "text-warn-300 font-medium"
                                : "text-dark-100"
                            }`}
                          >
                            {entry.is_dir ? (
                              <svg className="w-4 h-4 text-warn-400 shrink-0" fill="currentColor" viewBox="0 0 20 20">
                                <path d="M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
                              </svg>
                            ) : isArch ? (
                              <svg className="w-4 h-4 text-warn-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                                <path strokeLinecap="round" strokeLinejoin="round" d="M20.25 7.5l-.625 10.632a2.25 2.25 0 01-2.247 2.118H6.622a2.25 2.25 0 01-2.247-2.118L3.75 7.5M10 11.25h4M3.375 7.5h17.25c.621 0 1.125-.504 1.125-1.125v-1.5c0-.621-.504-1.125-1.125-1.125H3.375c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125z" />
                              </svg>
                            ) : (
                              <svg className="w-4 h-4 text-dark-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                                <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z" />
                              </svg>
                            )}
                            <span className="truncate max-w-xs">{entry.name}</span>
                          </button>
                        </td>

                        {/* Size */}
                        <td className="px-3 py-2.5 text-dark-300">
                          {entry.is_dir ? "—" : formatSize(entry.size)}
                        </td>

                        {/* Permissions */}
                        <td className="px-3 py-2.5">
                          <button
                            onClick={() => {
                              setChmodTarget(entry.name);
                              setChmodMode(entry.permissions || "0755");
                              setChmodRecursive(entry.is_dir);
                            }}
                            className="px-1.5 py-0.5 rounded bg-dark-700 hover:bg-dark-600 text-dark-200 text-[11px] font-mono border border-dark-600"
                            title="Click to change permissions"
                          >
                            {entry.permissions || "0755"}
                          </button>
                        </td>

                        {/* Owner */}
                        <td className="px-3 py-2.5">
                          <button
                            onClick={() => {
                              setChownTarget(entry.name);
                              setChownOwner(entry.owner || "www-data:www-data");
                              setChownRecursive(entry.is_dir);
                            }}
                            className="px-1.5 py-0.5 rounded bg-dark-700/60 hover:bg-dark-600 text-dark-300 text-[11px] font-mono truncate max-w-[100px]"
                            title="Click to change ownership"
                          >
                            {entry.owner || "www-data"}
                          </button>
                        </td>

                        {/* Modified */}
                        <td className="px-3 py-2.5 text-dark-400 text-[11px]">
                          {formatDate(entry.modified)}
                        </td>

                        {/* Row Actions */}
                        <td className="px-3 py-2.5 text-right">
                          <div className="flex items-center justify-end gap-1 opacity-70 group-hover:opacity-100 transition-opacity">
                            {/* Extract Button for Archives */}
                            {isArch && (
                              <button
                                onClick={() => {
                                  setExtractArchiveTarget(entry.name);
                                  setExtractDestination(currentPath);
                                }}
                                className="p-1 text-warn-400 hover:text-warn-300 hover:bg-dark-700 rounded transition-colors"
                                title="Extract archive"
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                                  <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
                                </svg>
                              </button>
                            )}

                            {/* Edit Button for Editable Files */}
                            {!entry.is_dir && isEdit && (
                              <button
                                onClick={() => openFile(entry.name)}
                                className="p-1 text-dark-300 hover:text-accent-400 hover:bg-dark-700 rounded transition-colors"
                                title="Edit file"
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                                  <path strokeLinecap="round" strokeLinejoin="round" d="m16.862 4.487 1.687-1.688a1.875 1.875 0 112.652 2.652L10.582 16.07a4.5 4.5 0 01-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 011.13-1.897l8.932-8.931Zm0 0L19.5 7.125" />
                                </svg>
                              </button>
                            )}

                            {/* Download Button */}
                            {!entry.is_dir && (
                              <a
                                href={`/api/sites/${id}/files/download?path=${encodeURIComponent(
                                  (currentPath === "." ? "" : currentPath + "/") + entry.name
                                )}`}
                                className="p-1 text-dark-300 hover:text-dark-50 hover:bg-dark-700 rounded transition-colors"
                                title="Download"
                                download
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                                  <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5M16.5 12L12 16.5m0 0L7.5 12m4.5 4.5V3" />
                                </svg>
                              </a>
                            )}

                            {/* Rename Button */}
                            <button
                              onClick={() => {
                                setRenaming(entry.name);
                                setRenameTo(entry.name);
                              }}
                              className="p-1 text-dark-300 hover:text-dark-200 hover:bg-dark-700 rounded transition-colors"
                              title="Rename"
                            >
                              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                                <path strokeLinecap="round" strokeLinejoin="round" d="M7.5 21L3 16.5m0 0L7.5 12M3 16.5h13.5m0-13.5L21 7.5m0 0L16.5 12M21 7.5H7.5" />
                              </svg>
                            </button>

                            {/* Delete Button */}
                            <button
                              onClick={() => setDeleteTargets([entry.name])}
                              className="p-1 text-dark-300 hover:text-danger-500 hover:bg-dark-700 rounded transition-colors"
                              title="Delete"
                            >
                              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                                <path strokeLinecap="round" strokeLinejoin="round" d="m14.74 9-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
                              </svg>
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </div>

        {/* Editor Panel */}
        {editing && (
          <div
            className={`bg-dark-800 rounded-lg border border-dark-600 flex flex-col overflow-hidden shadow-2xl ${
              editorFullscreen
                ? "fixed inset-4 z-50 bg-dark-900 border-rust-500/50"
                : "w-1/2 h-[650px]"
            }`}
          >
            {/* Editor Top Bar */}
            <div className="px-4 py-2.5 bg-dark-900 border-b border-dark-600 flex items-center justify-between shrink-0 font-mono text-xs">
              <div className="flex items-center gap-2 overflow-hidden">
                <svg className="w-4 h-4 text-accent-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z" />
                </svg>
                <span className="text-dark-100 font-semibold truncate">
                  {editing}
                </span>
                {saveSuccess && (
                  <span className="text-emerald-400 font-semibold text-[11px] animate-fade-in">
                    ✓ Saved
                  </span>
                )}
              </div>

              <div className="flex items-center gap-2 shrink-0">
                <button
                  onClick={saveFile}
                  disabled={saving}
                  className="px-3 py-1 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium transition-colors flex items-center gap-1.5 disabled:opacity-50"
                  title="Save (Ctrl+S / Cmd+S)"
                >
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M4.5 12.75l6 6 9-13.5" />
                  </svg>
                  {saving ? "Saving..." : "Save"}
                </button>

                <button
                  onClick={() => setEditorFullscreen((v) => !v)}
                  className="p-1 text-dark-300 hover:text-dark-100 rounded transition-colors"
                  title={editorFullscreen ? "Exit Fullscreen" : "Fullscreen"}
                >
                  {editorFullscreen ? (
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M9 9V4.5M9 9H4.5M9 9L3.75 3.75M9 15v4.5M9 15H4.5M9 15l-5.25 5.25M15 9h4.5M15 9V4.5M15 9l5.25-5.25M15 15h4.5M15 15v4.5m0-4.5l5.25 5.25" />
                    </svg>
                  ) : (
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M3.75 3.75v4.5m0-4.5h4.5m-4.5 0L9 9M3.75 20.25v-4.5m0 4.5h4.5m-4.5 0L9 15M20.25 3.75h-4.5m4.5 0v4.5m0-4.5L15 9m5.25 11.25h-4.5m4.5 0v-4.5m0 4.5L15 15" />
                    </svg>
                  )}
                </button>

                <button
                  onClick={() => {
                    setEditing(null);
                    setEditorFullscreen(false);
                  }}
                  className="p-1 text-dark-300 hover:text-dark-100 rounded transition-colors"
                  title="Close editor"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </div>
            </div>

            {/* Textarea */}
            <textarea
              ref={editorRef}
              value={editorContent}
              onChange={(e) => setEditorContent(e.target.value)}
              onKeyDown={(e) => {
                if ((e.ctrlKey || e.metaKey) && e.key === "s") {
                  e.preventDefault();
                  saveFile();
                } else if (e.key === "Tab") {
                  e.preventDefault();
                  const target = e.currentTarget;
                  const start = target.selectionStart;
                  const end = target.selectionEnd;
                  const val = target.value;
                  setEditorContent(val.substring(0, start) + "  " + val.substring(end));
                  setTimeout(() => {
                    target.selectionStart = target.selectionEnd = start + 2;
                  }, 0);
                }
              }}
              className="flex-1 p-4 font-mono text-xs text-dark-50 bg-dark-950 resize-none focus:outline-none custom-scrollbar leading-relaxed"
              spellCheck={false}
            />
          </div>
        )}
      </div>

      {/* Extract Modal */}
      {extractArchiveTarget && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 dp-modal-overlay">
          <div className="bg-dark-800 rounded-lg shadow-2xl p-6 w-[450px] border border-dark-600 dp-modal animate-fade-in font-mono">
            <h3 className="text-xs font-semibold text-warn-400 uppercase tracking-wider mb-2 flex items-center gap-2">
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
              </svg>
              Extract Archive
            </h3>
            <p className="text-xs text-dark-300 mb-4 break-all">
              Extract <span className="text-dark-100 font-semibold">{extractArchiveTarget}</span>
            </p>

            <label className="block text-xs text-dark-300 mb-1">
              Destination Directory (relative to site root):
            </label>
            <input
              type="text"
              value={extractDestination}
              onChange={(e) => setExtractDestination(e.target.value)}
              placeholder="e.g. public or ."
              className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-xs text-dark-100 focus:ring-2 focus:ring-rust-500 focus:border-rust-500 mb-3"
            />
            <div className="flex gap-2 mb-4 text-[11px]">
              <button
                type="button"
                onClick={() => setExtractDestination(currentPath)}
                className="text-rust-400 hover:text-rust-300 underline"
              >
                Use current folder ({currentPath})
              </button>
              <span className="text-dark-500">•</span>
              <button
                type="button"
                onClick={() => setExtractDestination("public")}
                className="text-rust-400 hover:text-rust-300 underline"
              >
                Use public/
              </button>
            </div>

            <p className="text-[11px] text-dark-400 mb-4">
              Note: Extracted files will automatically have correct <code className="text-warn-400">www-data:www-data</code> permissions.
            </p>

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setExtractArchiveTarget(null)}
                disabled={extracting}
                className="px-4 py-2 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                onClick={handleExtract}
                disabled={extracting}
                className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded-lg text-xs font-medium disabled:opacity-50 flex items-center gap-1.5"
              >
                {extracting ? (
                  <>
                    <div className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    Extracting...
                  </>
                ) : (
                  "Extract Here"
                )}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Compress Modal */}
      {showCompress && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 dp-modal-overlay">
          <div className="bg-dark-800 rounded-lg shadow-2xl p-6 w-[450px] border border-dark-600 dp-modal animate-fade-in font-mono">
            <h3 className="text-xs font-semibold text-rust-400 uppercase tracking-wider mb-2">
              Compress {selected.size} Item(s)
            </h3>
            <p className="text-xs text-dark-300 mb-4">
              Selected: {Array.from(selected).slice(0, 3).join(", ")}
              {selected.size > 3 ? ` and ${selected.size - 3} more` : ""}
            </p>

            <label className="block text-xs text-dark-300 mb-1">Archive Format:</label>
            <div className="flex gap-2 mb-4">
              <button
                type="button"
                onClick={() => {
                  setCompressFormat("zip");
                  setCompressName((prev) => prev.replace(/\.tar\.gz$/, ".zip"));
                }}
                className={`flex-1 py-1.5 rounded text-xs font-medium transition-colors ${
                  compressFormat === "zip"
                    ? "bg-rust-500 text-white"
                    : "bg-dark-700 text-dark-200 hover:bg-dark-600"
                }`}
              >
                .zip (Standard)
              </button>
              <button
                type="button"
                onClick={() => {
                  setCompressFormat("tar.gz");
                  setCompressName((prev) => prev.replace(/\.zip$/, ".tar.gz"));
                }}
                className={`flex-1 py-1.5 rounded text-xs font-medium transition-colors ${
                  compressFormat === "tar.gz"
                    ? "bg-rust-500 text-white"
                    : "bg-dark-700 text-dark-200 hover:bg-dark-600"
                }`}
              >
                .tar.gz (Gzipped)
              </button>
            </div>

            <label className="block text-xs text-dark-300 mb-1">Archive File Name:</label>
            <input
              type="text"
              value={compressName}
              onChange={(e) => setCompressName(e.target.value)}
              className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-xs text-dark-100 focus:ring-2 focus:ring-rust-500 mb-4"
            />

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setShowCompress(false)}
                disabled={compressing}
                className="px-4 py-2 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                onClick={handleCompress}
                disabled={compressing || !compressName.trim()}
                className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded-lg text-xs font-medium disabled:opacity-50 flex items-center gap-1.5"
              >
                {compressing ? (
                  <>
                    <div className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    Compressing...
                  </>
                ) : (
                  "Compress"
                )}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Copy / Move Modal */}
      {copyMoveAction && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 dp-modal-overlay">
          <div className="bg-dark-800 rounded-lg shadow-2xl p-6 w-[450px] border border-dark-600 dp-modal animate-fade-in font-mono">
            <h3 className="text-xs font-semibold text-dark-100 uppercase tracking-wider mb-2">
              {copyMoveAction === "copy" ? "Copy" : "Move"} {selected.size} Item(s)
            </h3>
            <p className="text-xs text-dark-300 mb-4">
              Destination directory (relative to site root):
            </p>

            <input
              type="text"
              value={copyMoveDestination}
              onChange={(e) => setCopyMoveDestination(e.target.value)}
              placeholder="e.g. public/backup or ."
              className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-xs text-dark-100 focus:ring-2 focus:ring-rust-500 mb-4"
            />

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setCopyMoveAction(null)}
                disabled={copyMoveLoading}
                className="px-4 py-2 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                onClick={handleCopyMove}
                disabled={copyMoveLoading}
                className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded-lg text-xs font-medium disabled:opacity-50 flex items-center gap-1.5"
              >
                {copyMoveLoading ? (
                  <>
                    <div className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    Processing...
                  </>
                ) : (
                  copyMoveAction === "copy" ? "Copy Here" : "Move Here"
                )}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Chmod Modal */}
      {chmodTarget && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 dp-modal-overlay">
          <div className="bg-dark-800 rounded-lg shadow-2xl p-6 w-96 border border-dark-600 dp-modal animate-fade-in font-mono">
            <h3 className="text-xs font-semibold text-dark-100 uppercase tracking-wider mb-2">
              Change Permissions
            </h3>
            <p className="text-xs text-dark-300 mb-4 truncate">
              Target: <span className="text-dark-100 font-semibold">{chmodTarget}</span>
            </p>

            <label className="block text-xs text-dark-300 mb-1">Octal Mode (e.g. 0755, 0644):</label>
            <input
              type="text"
              value={chmodMode}
              onChange={(e) => setChmodMode(e.target.value)}
              className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-xs text-dark-100 focus:ring-2 focus:ring-rust-500 mb-3"
            />

            <div className="grid grid-cols-2 gap-1.5 mb-4 text-[11px]">
              <button
                type="button"
                onClick={() => setChmodMode("0755")}
                className="py-1 px-2 bg-dark-700 hover:bg-dark-600 rounded text-dark-200 text-center"
              >
                0755 (Standard Dir)
              </button>
              <button
                type="button"
                onClick={() => setChmodMode("0644")}
                className="py-1 px-2 bg-dark-700 hover:bg-dark-600 rounded text-dark-200 text-center"
              >
                0644 (Standard File)
              </button>
              <button
                type="button"
                onClick={() => setChmodMode("0640")}
                className="py-1 px-2 bg-dark-700 hover:bg-dark-600 rounded text-dark-200 text-center"
              >
                0640 (Protected Config)
              </button>
              <button
                type="button"
                onClick={() => setChmodMode("0777")}
                className="py-1 px-2 bg-dark-700 hover:bg-dark-600 rounded text-warn-400 text-center"
              >
                0777 (Full Access)
              </button>
            </div>

            <label className="flex items-center gap-2 text-xs text-dark-300 mb-4 cursor-pointer">
              <input
                type="checkbox"
                checked={chmodRecursive}
                onChange={(e) => setChmodRecursive(e.target.checked)}
                className="rounded border-dark-500 bg-dark-700 text-rust-500"
              />
              Apply recursively (-R) to all subitems
            </label>

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setChmodTarget(null)}
                disabled={chmodLoading}
                className="px-4 py-2 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                onClick={handleChmod}
                disabled={chmodLoading}
                className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded-lg text-xs font-medium disabled:opacity-50"
              >
                {chmodLoading ? "Applying..." : "Apply Permissions"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Chown Modal */}
      {chownTarget && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 dp-modal-overlay">
          <div className="bg-dark-800 rounded-lg shadow-2xl p-6 w-96 border border-dark-600 dp-modal animate-fade-in font-mono">
            <h3 className="text-xs font-semibold text-dark-100 uppercase tracking-wider mb-2">
              Change Ownership
            </h3>
            <p className="text-xs text-dark-300 mb-4 truncate">
              Target: <span className="text-dark-100 font-semibold">{chownTarget}</span>
            </p>

            <label className="block text-xs text-dark-300 mb-1">User:Group (e.g. www-data:www-data):</label>
            <input
              type="text"
              value={chownOwner}
              onChange={(e) => setChownOwner(e.target.value)}
              className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-xs text-dark-100 focus:ring-2 focus:ring-rust-500 mb-3"
            />

            <div className="flex gap-2 mb-4 text-[11px]">
              <button
                type="button"
                onClick={() => setChownOwner("www-data:www-data")}
                className="flex-1 py-1 px-2 bg-dark-700 hover:bg-dark-600 rounded text-dark-200 text-center"
              >
                www-data:www-data (Web)
              </button>
              <button
                type="button"
                onClick={() => setChownOwner("root:root")}
                className="flex-1 py-1 px-2 bg-dark-700 hover:bg-dark-600 rounded text-dark-200 text-center"
              >
                root:root (System)
              </button>
            </div>

            <label className="flex items-center gap-2 text-xs text-dark-300 mb-4 cursor-pointer">
              <input
                type="checkbox"
                checked={chownRecursive}
                onChange={(e) => setChownRecursive(e.target.checked)}
                className="rounded border-dark-500 bg-dark-700 text-rust-500"
              />
              Apply recursively (-R) to all subitems
            </label>

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setChownTarget(null)}
                disabled={chownLoading}
                className="px-4 py-2 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                onClick={handleChown}
                disabled={chownLoading}
                className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded-lg text-xs font-medium disabled:opacity-50"
              >
                {chownLoading ? "Applying..." : "Apply Ownership"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Create Modal */}
      {showCreate && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 dp-modal-overlay">
          <div className="bg-dark-800 rounded-lg shadow-2xl p-6 w-96 border border-dark-600 dp-modal animate-fade-in font-mono">
            <h3 className="text-xs font-medium text-dark-300 uppercase font-mono tracking-widest mb-4">
              Create New
            </h3>
            <div className="flex gap-2 mb-4">
              <button
                type="button"
                onClick={() => setCreateType("file")}
                className={`flex-1 py-2 rounded-lg text-xs font-medium transition-colors ${
                  createType === "file"
                    ? "bg-rust-500 text-white"
                    : "bg-dark-700 text-dark-200 hover:bg-dark-600"
                }`}
              >
                File
              </button>
              <button
                type="button"
                onClick={() => setCreateType("dir")}
                className={`flex-1 py-2 rounded-lg text-xs font-medium transition-colors ${
                  createType === "dir"
                    ? "bg-rust-500 text-white"
                    : "bg-dark-700 text-dark-200 hover:bg-dark-600"
                }`}
              >
                Folder
              </button>
            </div>
            <input
              type="text"
              value={createName}
              onChange={(e) => setCreateName(e.target.value)}
              placeholder={createType === "dir" ? "folder-name" : "filename.txt"}
              className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-xs text-dark-100 focus:ring-2 focus:ring-rust-500"
              autoFocus
              onKeyDown={(e) => e.key === "Enter" && handleCreate()}
            />
            <div className="flex justify-end gap-2 mt-4">
              <button
                onClick={() => {
                  setShowCreate(false);
                  setCreateName("");
                }}
                disabled={creating}
                className="px-4 py-2 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                onClick={handleCreate}
                disabled={creating || !createName.trim()}
                className="px-4 py-2 bg-rust-500 text-white rounded-lg text-xs font-medium hover:bg-rust-600 disabled:opacity-50"
              >
                {creating ? "Creating..." : "Create"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Rename Modal */}
      {renaming && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 dp-modal-overlay">
          <div className="bg-dark-800 rounded-lg shadow-2xl p-6 w-96 border border-dark-600 dp-modal animate-fade-in font-mono">
            <h3 className="text-xs font-medium text-dark-300 uppercase font-mono tracking-widest mb-4">
              Rename "{renaming}"
            </h3>
            <input
              type="text"
              value={renameTo}
              onChange={(e) => setRenameTo(e.target.value)}
              className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-xs text-dark-100 focus:ring-2 focus:ring-rust-500"
              autoFocus
              onKeyDown={(e) => e.key === "Enter" && handleRename()}
            />
            <div className="flex justify-end gap-2 mt-4">
              <button
                onClick={() => setRenaming(null)}
                disabled={renamingLoading}
                className="px-4 py-2 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                onClick={handleRename}
                disabled={renamingLoading || !renameTo.trim()}
                className="px-4 py-2 bg-rust-500 text-white rounded-lg text-xs font-medium hover:bg-rust-600 disabled:opacity-50"
              >
                {renamingLoading ? "Renaming..." : "Rename"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Delete Confirmation Modal */}
      {deleteTargets.length > 0 && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 dp-modal-overlay">
          <div className="bg-dark-800 rounded-lg shadow-2xl p-6 w-96 border border-dark-600 dp-modal animate-fade-in font-mono">
            <h3 className="text-xs font-medium text-danger-400 uppercase font-mono tracking-widest mb-2 flex items-center gap-2">
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
              </svg>
              Delete {deleteTargets.length === 1 ? `"${deleteTargets[0]}"` : `${deleteTargets.length} items`}?
            </h3>
            <p className="text-xs text-dark-300 mb-4">
              This action cannot be undone. Files and directories will be permanently removed.
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setDeleteTargets([])}
                disabled={deleting}
                className="px-4 py-2 text-xs text-dark-300 hover:text-dark-100"
              >
                Cancel
              </button>
              <button
                onClick={handleDeleteConfirmed}
                disabled={deleting}
                className="px-4 py-2 bg-danger-500 text-white rounded-lg text-xs font-medium hover:bg-danger-600 disabled:opacity-50 flex items-center gap-1.5"
              >
                {deleting ? (
                  <>
                    <div className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    Deleting...
                  </>
                ) : (
                  "Delete Permanently"
                )}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
