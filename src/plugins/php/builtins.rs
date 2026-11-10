//! A small table of PHP built-in classes. They are outside the project index, so calls
//! to their methods are `external`; the table gives the declaring class and return type.

struct Builtin {
    name: &'static str,
    parents: &'static [&'static str],
    methods: &'static [(&'static str, &'static str)],
}

const THROWABLE: &[(&str, &str)] = &[
    ("getMessage", "string"),
    ("getCode", "int"),
    ("getFile", "string"),
    ("getLine", "int"),
    ("getTrace", "array"),
    ("getTraceAsString", "string"),
    ("getPrevious", "Throwable|null"),
    ("__toString", "string"),
];
const DATE_TIME: &[(&str, &str)] = &[
    ("format", "string"),
    ("getTimestamp", "int"),
    ("getTimezone", "DateTimeZone|false"),
    ("getOffset", "int"),
    ("diff", "DateInterval"),
];
const MUTABLE_DATE: &[(&str, &str)] = &[
    ("modify", "static|false"),
    ("setDate", "static"),
    ("setTime", "static"),
    ("setTimestamp", "static"),
    ("setTimezone", "static"),
    ("add", "static"),
    ("sub", "static"),
];

const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "Throwable",
        parents: &["Stringable"],
        methods: THROWABLE,
    },
    Builtin {
        name: "Exception",
        parents: &["Throwable"],
        methods: THROWABLE,
    },
    Builtin {
        name: "Error",
        parents: &["Throwable"],
        methods: THROWABLE,
    },
    Builtin {
        name: "ErrorException",
        parents: &["Exception"],
        methods: &[("getSeverity", "int")],
    },
    Builtin {
        name: "DateTimeInterface",
        parents: &[],
        methods: DATE_TIME,
    },
    Builtin {
        name: "DateTime",
        parents: &["DateTimeInterface"],
        methods: MUTABLE_DATE,
    },
    Builtin {
        name: "DateTimeImmutable",
        parents: &["DateTimeInterface"],
        methods: MUTABLE_DATE,
    },
    Builtin {
        name: "DateTimeZone",
        parents: &[],
        methods: &[("getName", "string"), ("getOffset", "int")],
    },
    Builtin {
        name: "DateInterval",
        parents: &[],
        methods: &[("format", "string")],
    },
    Builtin {
        name: "Closure",
        parents: &[],
        methods: &[
            ("bind", "Closure|null"),
            ("bindTo", "Closure|null"),
            ("call", "mixed"),
            ("__invoke", "mixed"),
            ("fromCallable", "Closure"),
        ],
    },
    Builtin {
        name: "Countable",
        parents: &[],
        methods: &[("count", "int")],
    },
    Builtin {
        name: "IteratorAggregate",
        parents: &[],
        methods: &[("getIterator", "Traversable")],
    },
    Builtin {
        name: "JsonSerializable",
        parents: &[],
        methods: &[("jsonSerialize", "mixed")],
    },
    Builtin {
        name: "Stringable",
        parents: &[],
        methods: &[("__toString", "string")],
    },
    Builtin {
        name: "ArrayObject",
        parents: &["IteratorAggregate", "Countable"],
        methods: &[
            ("getArrayCopy", "array"),
            ("offsetExists", "bool"),
            ("offsetGet", "mixed"),
            ("offsetSet", "void"),
            ("offsetUnset", "void"),
            ("append", "void"),
        ],
    },
    Builtin {
        name: "ArrayIterator",
        parents: &["Countable"],
        methods: &[
            ("getArrayCopy", "array"),
            ("current", "mixed"),
            ("key", "string|int|null"),
            ("next", "void"),
            ("valid", "bool"),
        ],
    },
];

/// Standard exceptions and errors without methods of their own.
const EXCEPTIONS: &[(&str, &str)] = &[
    ("RuntimeException", "Exception"),
    ("LogicException", "Exception"),
    ("JsonException", "Exception"),
    ("InvalidArgumentException", "LogicException"),
    ("DomainException", "LogicException"),
    ("LengthException", "LogicException"),
    ("OutOfRangeException", "LogicException"),
    ("BadFunctionCallException", "LogicException"),
    ("BadMethodCallException", "BadFunctionCallException"),
    ("OutOfBoundsException", "RuntimeException"),
    ("RangeException", "RuntimeException"),
    ("OverflowException", "RuntimeException"),
    ("UnderflowException", "RuntimeException"),
    ("UnexpectedValueException", "RuntimeException"),
    ("TypeError", "Error"),
    ("ValueError", "Error"),
    ("ArithmeticError", "Error"),
    ("DivisionByZeroError", "ArithmeticError"),
];

fn parents(class: &str) -> Vec<&'static str> {
    if let Some((_, parent)) = EXCEPTIONS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(class))
    {
        return vec![parent];
    }
    BUILTINS
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(class))
        .map(|b| b.parents.to_vec())
        .unwrap_or_default()
}

pub(super) fn is_builtin(class: &str) -> bool {
    let class = class.trim_start_matches('\\');
    EXCEPTIONS
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(class))
        || BUILTINS.iter().any(|b| b.name.eq_ignore_ascii_case(class))
}

/// `(declaring class, return type)` of `member` on built-in `class` or its parents.
pub(super) fn method(class: &str, member: &str) -> Option<(&'static str, &'static str)> {
    let mut pending = vec![class.trim_start_matches('\\').to_owned()];
    let mut seen = Vec::new();
    while let Some(current) = pending.pop() {
        if seen.contains(&current.to_ascii_lowercase()) {
            continue;
        }
        seen.push(current.to_ascii_lowercase());
        if let Some(builtin) = BUILTINS
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(&current))
        {
            if let Some((_, returns)) = builtin
                .methods
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(member))
            {
                return Some((builtin.name, returns));
            }
        }
        pending.extend(parents(&current).into_iter().map(str::to_owned));
    }
    None
}
