"""Optional journal-line construction checks beyond the packaged cents overlay.

Use LedgerLine for nonnegative, single-sided journal lines. The outer package's
build overlay already enforces strict signed i64 cents in ordinary LineRequest.
The server and database remain authoritative for accounting invariants.
"""
from typing import Annotated

from pydantic import Field, StrictInt, model_validator

from ..models.coolerp import LineRequest

type NonNegativeCents = Annotated[StrictInt, Field(ge=0, le=2**63 - 1)]


class LedgerLine(LineRequest):
    debit: NonNegativeCents
    credit: NonNegativeCents

    @model_validator(mode="after")
    def one_side(self) -> "LedgerLine":
        if (self.debit == 0) == (self.credit == 0):
            raise ValueError("exactly one of debit and credit must be positive")
        return self
