"""CoolERP's signed i64 cents, with no coercion from Python/JSON floats."""
from typing import Annotated

from pydantic import Field, StrictInt

type Cents = Annotated[StrictInt, Field(ge=-(2**63), le=2**63 - 1)]
