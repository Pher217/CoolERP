/// Read ledger entries.
pub const LEDGER_READ: &str = "ledger:read";
/// Post journal entries.
pub const LEDGER_POST: &str = "ledger:post";
/// Create / update AR invoices.
pub const AR_INVOICE: &str = "ar:invoice";
/// Create / update AP bills.
pub const AP_BILL: &str = "ap:bill";
/// Register payments.
pub const PAYMENT_REGISTER: &str = "payment:register";
/// Record inventory movements.
pub const INVENTORY_MOVE: &str = "inventory:move";
/// Create / update parties (customers, vendors, …).
pub const PARTY_WRITE: &str = "party:write";
/// Create / update chart-of-accounts entries.
pub const ACCOUNT_WRITE: &str = "account:write";
/// Read the append-only audit log.
pub const AUDIT_READ: &str = "audit:read";

/// All defined scopes in declaration order.
pub const ALL: &[&str] = &[
    LEDGER_READ,
    LEDGER_POST,
    AR_INVOICE,
    AP_BILL,
    PAYMENT_REGISTER,
    INVENTORY_MOVE,
    PARTY_WRITE,
    ACCOUNT_WRITE,
    AUDIT_READ,
];
