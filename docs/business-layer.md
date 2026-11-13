# Headless desktop business layer

`ramflux-desktop-sdk::business` prepares reads and applies the existing core policy
without depending on GPUI. It preserves the daemon connection, one source per
account, source-owned retries and Store's single-writer event application.
It does not open another account runtime or database.

`actions(store, intent, now)` returns executable effects or a pending human
confirmation. A confirmation is retained as its original intent and cannot be
converted into a daemon request. Executable effects use the existing planner;
the source remains responsible for admission, completion, retry identity and
reconciliation. Planning is not evidence that an operation succeeded.

| Area | Existing adapter/policy | New presentation/read seam |
| --- | --- | --- |
| Conversations | Send/start, mute, disappear, pin, history and source receipts | Full paged directory, title/member/cached-body search, activity for unloaded histories, unread/attention and outgoing receipts |
| Contacts/strangers | Request accept/decline/block, contact links, safety verification and request settings | Pending-request totals; bounded batch validation preserving each block confirmation |
| Groups/roles | Create/rename, membership, invitations and role changes | Directory/identity/contact read plan; existing governance policy and daemon authority retained |
| Agents | Owner custody ceremony and one-shot credential export | Runtime/tool/scope draft validation against an explicit offered catalog; no default permissions |
| Devices | List/revoke and existing sync capability refusals | Exact-device platform/origin facts, browser operator warning and missing-origin state |
| Backup/settings | Full backup/restore job lifecycle, account/node settings | Ordered authoritative reads; existing secrets and job policies retained |

Conversation previews borrow only currently cached text. Deleted content is
excluded from both preview and search. Locked accounts expose no directory rows.
The full directory is distinct from the six-entry context stack. Query offsets
apply to one snapshot and must reset after updates. This is local search over
loaded facts, not a durable or server full-text index. Reading does not mark a
conversation read. Directory timestamps are Unix seconds converted through the
same saturating conversion as message timestamps.

Request batches validate the complete selection before returning effects. A
stale, duplicate or expired row refuses the batch before submission. Effects
are sequential; the daemon does not promise an atomic batch transaction.

`ReadPlan::gaps` explicitly identifies contracts absent from the pinned daemon:
dedicated Agent configuration, authenticated Web-device origin facts, and
stranger reply/report/batch outcomes. Agent drafts and Web warnings are pure
models until those source facts/operations are published. A successful account
or device list is not acceptance of these missing flows. Reply/report must not
be emulated with contact acceptance or an unrelated message submission.

The product's separate `ramflux-client-model` crate defines a UI-independent
borrowed directory row/page, unread/attention totals and receipt presentation.
Its API can serve mobile adapters without a GPUI or desktop dependency. Consumer
pinning and replacement of desktop-local projection code are a separate
integration step once that product commit is published.
