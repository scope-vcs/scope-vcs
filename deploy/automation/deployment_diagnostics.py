from contextlib import contextmanager
import re


class OperationFailure(RuntimeError):
    def __init__(self, operation, error):
        if not re.fullmatch(r"[a-z][a-z0-9_.-]{0,79}", operation):
            raise ValueError("Invalid diagnostic operation")
        self.operation = operation
        self.category = type(error).__name__[:60]
        super().__init__(f"{self.operation}: {self.category}")


@contextmanager
def operation(name):
    try:
        yield
    except OperationFailure:
        raise
    except Exception as error:
        raise OperationFailure(name, error) from None


def diagnostic(error):
    return {"operation": error.operation, "category": error.category} if isinstance(error, OperationFailure) else {
        "operation": "session.lifecycle", "category": type(error).__name__[:60]}
