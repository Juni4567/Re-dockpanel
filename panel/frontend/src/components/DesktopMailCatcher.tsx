import { useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { api } from "../api";

interface MailCatcherStatus {
  running: boolean;
  smtp_host: string;
  smtp_port: number | null;
  message_count: number;
  unread_count: number;
}

interface CapturedEmailSummary {
  id: string;
  from: string;
  to: string[];
  subject: string;
  snippet: string;
  has_html: boolean;
  attachment_count: number;
  size_bytes: number;
  read: boolean;
  site_domain?: string | null;
  received_at: string;
}

interface EmailAttachment {
  filename: string;
  content_type: string;
  size_bytes: number;
}

interface CapturedEmailDetail {
  id: string;
  from: string;
  to: string[];
  cc: string[];
  subject: string;
  text_body: string;
  html_body: string;
  raw: string;
  headers: Record<string, string>;
  attachments: EmailAttachment[];
  size_bytes: number;
  read: boolean;
  site_domain?: string | null;
  received_at: string;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function formatTimestamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString();
}

export default function DesktopMailCatcher() {
  const [searchParams, setSearchParams] = useSearchParams();
  const initialDomain = searchParams.get("domain") || "";

  const [status, setStatus] = useState<MailCatcherStatus | null>(null);
  const [messages, setMessages] = useState<CapturedEmailSummary[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selected, setSelected] = useState<CapturedEmailDetail | null>(null);
  const [viewTab, setViewTab] = useState<"html" | "text" | "headers" | "raw">("html");
  const [search, setSearch] = useState("");
  const [domainFilter, setDomainFilter] = useState(initialDomain);
  const [loading, setLoading] = useState(true);
  const [sendingTest, setSendingTest] = useState(false);
  const [feedback, setFeedback] = useState<{ text: string; type: "success" | "error" } | null>(null);

  useEffect(() => {
    const paramDomain = searchParams.get("domain") || "";
    if (paramDomain !== domainFilter) {
      setDomainFilter(paramDomain);
    }
  }, [searchParams]);

  const loadMessages = useCallback(async () => {
    try {
      const params = new URLSearchParams();
      if (search.trim()) params.set("q", search.trim());
      if (domainFilter.trim()) params.set("domain", domainFilter.trim());
      const qs = params.toString();
      const res = await api.get<{ messages: CapturedEmailSummary[]; status: MailCatcherStatus }>(
        `/desktop/mail/messages${qs ? `?${qs}` : ""}`,
      );
      setMessages(res.messages || []);
      if (res.status) setStatus(res.status);
    } catch (err: any) {
      setFeedback({ text: err.message || "Failed loading local inbox", type: "error" });
    } finally {
      setLoading(false);
    }
  }, [search, domainFilter]);

  useEffect(() => {
    loadMessages();
    const timer = setInterval(loadMessages, 3000);
    return () => clearInterval(timer);
  }, [loadMessages]);

  const openMessage = async (id: string) => {
    setSelectedId(id);
    try {
      const detail = await api.get<CapturedEmailDetail>(`/desktop/mail/messages/${id}`);
      setSelected(detail);
      setViewTab(detail.html_body.trim() ? "html" : "text");
      setMessages((prev) => prev.map((m) => (m.id === id ? { ...m, read: true } : m)));
      setStatus((prev) =>
        prev
          ? {
              ...prev,
              unread_count: Math.max(
                0,
                messages.filter((m) => !m.read && m.id !== id).length,
              ),
            }
          : prev,
      );
    } catch (err: any) {
      setFeedback({ text: err.message || "Failed reading email", type: "error" });
    }
  };

  const handleSendTest = async () => {
    setSendingTest(true);
    setFeedback(null);
    try {
      const res = await api.post<{ ok: boolean; smtp_port: number }>("/desktop/mail/test", {
        site_domain: domainFilter.trim() || undefined,
      });
      setFeedback({
        text: `Test email sent & captured on 127.0.0.1:${res.smtp_port}`,
        type: "success",
      });
      await loadMessages();
    } catch (err: any) {
      setFeedback({ text: err.message || "Failed sending test email", type: "error" });
    } finally {
      setSendingTest(false);
    }
  };

  const handleMarkAllRead = async () => {
    try {
      await api.post("/desktop/mail/messages/read-all", {});
      setMessages((prev) => prev.map((m) => ({ ...m, read: true })));
      if (selected) setSelected({ ...selected, read: true });
      setStatus((prev) => (prev ? { ...prev, unread_count: 0 } : prev));
    } catch (err: any) {
      setFeedback({ text: err.message || "Failed marking all read", type: "error" });
    }
  };

  const handleDeleteMessage = async (id: string) => {
    try {
      await api.delete(`/desktop/mail/messages/${id}`);
      if (selectedId === id) {
        setSelectedId(null);
        setSelected(null);
      }
      await loadMessages();
    } catch (err: any) {
      setFeedback({ text: err.message || "Failed deleting email", type: "error" });
    }
  };

  const handleClearAll = async () => {
    try {
      await api.delete("/desktop/mail/messages");
      setSelectedId(null);
      setSelected(null);
      await loadMessages();
      setFeedback({ text: "Local inbox cleared", type: "success" });
    } catch (err: any) {
      setFeedback({ text: err.message || "Failed clearing inbox", type: "error" });
    }
  };

  const clearDomainFilter = () => {
    setDomainFilter("");
    const next = new URLSearchParams(searchParams);
    next.delete("domain");
    setSearchParams(next);
  };

  return (
    <div className="p-6 max-w-7xl mx-auto space-y-5">
      {/* Header */}
      <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-3">
            <h1 className="text-xl font-bold text-dark-50 tracking-tight">Local Mail Catcher</h1>
            <span
              className={`inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-mono border ${
                status?.running
                  ? "bg-emerald-950/40 text-emerald-300 border-emerald-800/50"
                  : "bg-dark-800 text-dark-400 border-dark-700"
              }`}
            >
              <span
                className={`w-2 h-2 rounded-full ${status?.running ? "bg-emerald-400" : "bg-dark-500"}`}
              />
              {status?.running && status.smtp_port
                ? `SMTP ${status.smtp_host}:${status.smtp_port}`
                : "SMTP Offline"}
            </span>
            {(status?.unread_count ?? 0) > 0 && (
              <span className="px-2 py-0.5 rounded-full text-xs font-medium bg-rust-500/20 text-rust-300 border border-rust-500/30">
                {status?.unread_count} unread
              </span>
            )}
          </div>
          <p className="text-xs text-dark-300 mt-1">
            Captures outgoing PHP <code className="text-dark-100">mail()</code>, WordPress{" "}
            <code className="text-dark-100">wp_mail()</code>, and local SMTP traffic without sending real external mail.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={handleSendTest}
            disabled={sendingTest}
            className="px-3 py-1.5 bg-rust-500 hover:bg-rust-600 text-white text-xs font-medium rounded-lg transition-colors disabled:opacity-50"
          >
            {sendingTest ? "Sending..." : "Send Test Email"}
          </button>
          <button
            type="button"
            onClick={handleMarkAllRead}
            disabled={messages.length === 0}
            className="px-3 py-1.5 bg-dark-800 hover:bg-dark-700 text-dark-200 text-xs font-medium rounded-lg border border-dark-700 transition-colors disabled:opacity-40"
          >
            Mark All Read
          </button>
          <button
            type="button"
            onClick={handleClearAll}
            disabled={messages.length === 0}
            className="px-3 py-1.5 bg-dark-800 hover:bg-red-950/50 text-dark-200 hover:text-red-300 text-xs font-medium rounded-lg border border-dark-700 transition-colors disabled:opacity-40"
          >
            Clear Inbox
          </button>
          <button
            type="button"
            onClick={loadMessages}
            className="px-3 py-1.5 bg-dark-800 hover:bg-dark-700 text-dark-200 text-xs font-medium rounded-lg border border-dark-700 transition-colors"
          >
            Refresh
          </button>
        </div>
      </div>

      {feedback && (
        <div
          className={`px-4 py-2.5 rounded-lg text-xs border ${
            feedback.type === "success"
              ? "bg-emerald-950/40 border-emerald-800/40 text-emerald-300"
              : "bg-red-950/40 border-red-800/40 text-red-300"
          }`}
        >
          {feedback.text}
        </div>
      )}

      {/* Search & Site Filter */}
      <div className="flex flex-col sm:flex-row gap-3">
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search subject, sender, recipient, or body..."
          className="flex-1 px-3 py-2 bg-dark-900 border border-dark-700 rounded-lg text-sm text-dark-100 placeholder-dark-400 focus:outline-none focus:border-accent-500"
        />
        <div className="flex items-center gap-2">
          <input
            type="text"
            value={domainFilter}
            onChange={(e) => setDomainFilter(e.target.value)}
            placeholder="Filter by site domain..."
            className="w-56 px-3 py-2 bg-dark-900 border border-dark-700 rounded-lg text-sm text-dark-100 placeholder-dark-400 focus:outline-none focus:border-accent-500 font-mono"
          />
          {domainFilter && (
            <button
              type="button"
              onClick={clearDomainFilter}
              className="px-2.5 py-2 bg-dark-800 hover:bg-dark-700 text-dark-300 text-xs rounded-lg border border-dark-700"
            >
              Clear
            </button>
          )}
        </div>
      </div>

      {/* Split Inbox */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-4 min-h-[540px]">
        {/* Message List */}
        <div className="lg:col-span-5 bg-dark-900 border border-dark-800 rounded-xl overflow-hidden flex flex-col">
          <div className="px-4 py-2.5 border-b border-dark-800 flex items-center justify-between text-xs text-dark-300">
            <span className="font-medium uppercase tracking-wider font-mono">
              Inbox ({messages.length})
            </span>
          </div>
          <div className="flex-1 overflow-y-auto divide-y divide-dark-800/70 max-h-[600px]">
            {loading ? (
              <div className="p-6 text-center text-xs text-dark-400">Loading captured emails...</div>
            ) : messages.length === 0 ? (
              <div className="p-6 text-center space-y-3">
                <p className="text-sm text-dark-200 font-medium">No captured emails yet</p>
                <p className="text-xs text-dark-400 leading-relaxed">
                  PHP &amp; WordPress workers automatically route outgoing mail to{" "}
                  <code className="text-dark-200">
                    127.0.0.1:{status?.smtp_port ?? 1025}
                  </code>
                  . Trigger a password reset, contact form, or click <strong>Send Test Email</strong>.
                </p>
              </div>
            ) : (
              messages.map((m) => {
                const isActive = m.id === selectedId;
                return (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => openMessage(m.id)}
                    className={`w-full text-left px-4 py-3 transition-colors ${
                      isActive
                        ? "bg-dark-800/90 border-l-2 border-l-rust-500"
                        : "hover:bg-dark-800/50"
                    }`}
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2 min-w-0">
                        {!m.read && <span className="w-2 h-2 rounded-full bg-rust-400 shrink-0" />}
                        <span
                          className={`text-sm truncate ${
                            m.read ? "text-dark-200 font-normal" : "text-dark-50 font-semibold"
                          }`}
                        >
                          {m.subject}
                        </span>
                      </div>
                      <span className="text-[11px] text-dark-400 shrink-0 font-mono">
                        {formatBytes(m.size_bytes)}
                      </span>
                    </div>
                    <div className="flex items-center justify-between gap-2 mt-1 text-xs text-dark-400">
                      <span className="truncate font-mono">{m.from || "unknown"}</span>
                      {m.site_domain && (
                        <span className="px-1.5 py-0.5 rounded bg-dark-800 text-accent-300 text-[10px] font-mono shrink-0">
                          {m.site_domain}
                        </span>
                      )}
                    </div>
                    {m.snippet && (
                      <p className="text-xs text-dark-400 truncate mt-1">{m.snippet}</p>
                    )}
                  </button>
                );
              })
            )}
          </div>
        </div>

        {/* Message Viewer */}
        <div className="lg:col-span-7 bg-dark-900 border border-dark-800 rounded-xl overflow-hidden flex flex-col">
          {!selected ? (
            <div className="flex-1 flex flex-col items-center justify-center p-8 text-center text-dark-400 space-y-2">
              <p className="text-sm text-dark-200 font-medium">Select an email to inspect</p>
              <p className="text-xs max-w-md">
                View rendered HTML, plain text, SMTP headers, and raw MIME source captured locally.
              </p>
            </div>
          ) : (
            <>
              <div className="p-4 border-b border-dark-800 space-y-2">
                <div className="flex items-start justify-between gap-3">
                  <h2 className="text-base font-semibold text-dark-50 break-words">
                    {selected.subject}
                  </h2>
                  <button
                    type="button"
                    onClick={() => handleDeleteMessage(selected.id)}
                    className="px-2.5 py-1 text-xs bg-dark-800 hover:bg-red-950/60 text-dark-300 hover:text-red-300 rounded border border-dark-700 shrink-0"
                  >
                    Delete
                  </button>
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-1.5 text-xs text-dark-300">
                  <div>
                    <span className="text-dark-400">From: </span>
                    <span className="font-mono text-dark-100">{selected.from}</span>
                  </div>
                  <div>
                    <span className="text-dark-400">To: </span>
                    <span className="font-mono text-dark-100">{selected.to.join(", ")}</span>
                  </div>
                  {selected.cc.length > 0 && (
                    <div>
                      <span className="text-dark-400">Cc: </span>
                      <span className="font-mono text-dark-100">{selected.cc.join(", ")}</span>
                    </div>
                  )}
                  <div>
                    <span className="text-dark-400">Received: </span>
                    <span>{formatTimestamp(selected.received_at)}</span>
                  </div>
                </div>
                {selected.attachments.length > 0 && (
                  <div className="flex flex-wrap gap-1.5 pt-1">
                    {selected.attachments.map((att, idx) => (
                      <span
                        key={idx}
                        className="px-2 py-0.5 bg-dark-800 border border-dark-700 rounded text-[11px] font-mono text-dark-200"
                      >
                        {att.filename} ({att.content_type}, {formatBytes(att.size_bytes)})
                      </span>
                    ))}
                  </div>
                )}
              </div>

              {/* View Tabs */}
              <div className="px-4 py-2 border-b border-dark-800 flex items-center gap-2 bg-dark-950/40">
                {(
                  [
                    { id: "html", label: "HTML Preview", disabled: !selected.html_body.trim() },
                    { id: "text", label: "Plain Text", disabled: !selected.text_body.trim() },
                    { id: "headers", label: "Headers", disabled: false },
                    { id: "raw", label: "Raw Source", disabled: false },
                  ] as const
                ).map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    disabled={t.disabled}
                    onClick={() => setViewTab(t.id)}
                    className={`px-2.5 py-1 rounded text-xs font-medium transition-colors disabled:opacity-35 ${
                      viewTab === t.id
                        ? "bg-accent-600 text-white"
                        : "text-dark-300 hover:text-dark-100 hover:bg-dark-800"
                    }`}
                  >
                    {t.label}
                  </button>
                ))}
              </div>

              {/* Body */}
              <div className="flex-1 overflow-auto p-4">
                {viewTab === "html" && selected.html_body.trim() && (
                  <iframe
                    title="Captured Email HTML Preview"
                    sandbox=""
                    srcDoc={selected.html_body}
                    className="w-full h-[420px] rounded-lg bg-white border border-dark-700"
                  />
                )}
                {viewTab === "text" && (
                  <pre className="text-xs font-mono text-dark-100 whitespace-pre-wrap break-words leading-relaxed">
                    {selected.text_body || "(No plain text part)"}
                  </pre>
                )}
                {viewTab === "headers" && (
                  <div className="space-y-1.5 text-xs font-mono">
                    {Object.entries(selected.headers).map(([k, v]) => (
                      <div key={k} className="grid grid-cols-12 gap-2 border-b border-dark-800/50 pb-1">
                        <span className="col-span-4 text-accent-300 break-all">{k}</span>
                        <span className="col-span-8 text-dark-100 break-all">{v}</span>
                      </div>
                    ))}
                  </div>
                )}
                {viewTab === "raw" && (
                  <pre className="text-xs font-mono text-dark-200 whitespace-pre-wrap break-all leading-relaxed">
                    {selected.raw}
                  </pre>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
