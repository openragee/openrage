//! The public members of the runtime's library by the full name a call of one goes by, and the
//! shape each is declared with: on an object or on its type, a method, an accessor or a constructor.
//!
//! ⚠ What a member is comes from the library's own public declarations; the name a call goes by
//! comes only from a user's list, so a call no list names is never found here. A generic type's
//! count of parameters follows an underscore, as a list spells it: `Dictionary_2`.

use self::Form::{Get, Method, New, Open, OpenGet, OpenSet, Set, Static, StaticGet};

/// One set of overloads of a member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Form {
    /// A method of an object that no type a script declares can override, by the parameter counts of its public overloads.
    Method(&'static [u8]),
    /// A method of an object that a type a script declares may override.
    Open(&'static [u8]),
    /// A method of the type, by the parameter counts of its public overloads.
    Static(&'static [u8]),
    /// The getter of a property of an object.
    Get,
    /// The getter of a property of an object that a type a script declares may override.
    OpenGet,
    /// The setter of a property of an object.
    Set,
    /// The setter of a property of an object that a type a script declares may override.
    OpenSet,
    /// The getter of a property of the type.
    StaticGet,
    /// A constructor, by the parameter counts of its public overloads.
    New(&'static [u8]),
}

/// A type of the library: its full name, whether it is a value type, and its members.
pub(super) struct Type {
    pub name: &'static str,
    /// A value type's own members are handed the address of the value, never a reference to it.
    pub value: bool,
    pub members: &'static [(&'static str, Form)],
}

/// What a call of a member of the library reads as in C#.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reading {
    /// `receiver.Name(args)`, of the argument handed first.
    Method,
    /// `Type.Name`.
    StaticGetter,
    /// `new Type(args)`, of the object handed first.
    Constructor,
}

/// The overloads of one member that take a call's count of arguments.
#[derive(Default)]
struct Fits {
    method: bool,
    open: bool,
    accessor: bool,
    statics: bool,
    static_get: bool,
    made: bool,
}

/// What a call of the member `path` with `args` arguments reads as, where the table holds the member and exactly one reading takes that many.
// ⚠ An argument the runtime adds, such as a type's handle, makes a count no public overload has:
// the call is then left as it is.
pub(super) fn reading(path: &str, args: usize) -> Option<Reading> {
    let (owner, member) = path.rsplit_once('.')?;
    let ty = TYPES.iter().find(|ty| ty.name == owner)?;
    let takes = |counts: &[u8], receivers: usize| {
        let passed = args.checked_sub(receivers).map(u8::try_from);
        passed.is_some_and(|n| n.is_ok_and(|n| counts.contains(&n)))
    };
    let mut fits = Fits::default();
    for (_, form) in ty.members.iter().filter(|(name, _)| *name == member) {
        match *form {
            Form::Method(counts) => fits.method |= takes(counts, 1),
            Form::Open(counts) => fits.open |= takes(counts, 1),
            Form::Get => fits.accessor |= args == 1,
            Form::Set => fits.accessor |= args == 2,
            Form::OpenGet => fits.open |= args == 1,
            Form::OpenSet => fits.open |= args == 2,
            Form::Static(counts) => fits.statics |= takes(counts, 0),
            Form::StaticGet => fits.static_get |= args == 0,
            Form::New(counts) => fits.made |= takes(counts, 1),
        }
    }
    let on_object = fits.method || fits.open || fits.accessor;
    let on_type = fits.statics || fits.static_get;
    match (on_object, on_type, fits.made) {
        // An overload on an object and one on the type that take as many arguments: the call
        // does not say which it is.
        (true, true, _) => None,
        // ⚠ A method a script's type may override would read as the call of the override; an
        // accessor of an object is the `properties` reading's.
        (true, false, false) if !fits.open && !fits.accessor && !ty.value => Some(Reading::Method),
        (false, true, false) if !fits.statics => Some(Reading::StaticGetter),
        // ⚠ A value type is constructed in place, through its address.
        (false, false, true) if !ty.value => Some(Reading::Constructor),
        _ => None,
    }
}

/// Every type the table holds, each once.
pub(super) const TYPES: &[Type] = &[
    Type {
        name: "System.Object",
        value: false,
        members: &[
            ("Ctor", New(&[0])),
            ("GetType", Method(&[0])),
            ("GetHashCode", Open(&[0])),
            ("Equals", Open(&[1])),
            ("Equals", Static(&[2])),
            ("ReferenceEquals", Static(&[2])),
            ("ToString", Open(&[0])),
        ],
    },
    // The class every value type derives from: its members are handed the value boxed.
    Type {
        name: "System.ValueType",
        value: false,
        members: &[
            ("Equals", Open(&[1])),
            ("GetHashCode", Open(&[0])),
            ("ToString", Open(&[0])),
        ],
    },
    // ⚠ No type a script declares derives from `Delegate`: C# refuses one, and a delegate type
    // a compiler declares holds no method bodies of its own.
    Type {
        name: "System.Delegate",
        value: false,
        members: &[
            ("GetInvocationList", Method(&[0])),
            ("DynamicInvoke", Method(&[1])),
            ("Clone", Method(&[0])),
            ("get_Target", Get),
            ("get_Method", Get),
            ("Combine", Static(&[1, 2])),
            ("Remove", Static(&[2])),
            ("RemoveAll", Static(&[2])),
            ("CreateDelegate", Static(&[2, 3, 4])),
        ],
    },
    // An abstract class C# lets a script derive from.
    Type {
        name: "System.Type",
        value: false,
        members: &[
            ("GetType", Method(&[0])),
            ("GetType", Static(&[1, 2, 3])),
            ("GetTypeFromHandle", Static(&[1])),
            ("IsAssignableFrom", Open(&[1])),
            ("IsSubclassOf", Open(&[1])),
            ("IsInstanceOfType", Open(&[1])),
            ("IsEnumDefined", Open(&[1])),
            ("GetElementType", Open(&[0])),
            ("GetEnumUnderlyingType", Open(&[0])),
            ("GetGenericArguments", Open(&[0])),
            ("get_IsEnum", OpenGet),
            ("get_Name", OpenGet),
            ("get_FullName", OpenGet),
        ],
    },
    Type {
        name: "System.String",
        value: false,
        members: &[
            ("Clone", Method(&[0])),
            ("CompareTo", Method(&[1])),
            ("Compare", Static(&[2, 3, 4, 5, 6])),
            ("Contains", Method(&[1, 2])),
            ("CopyTo", Method(&[1, 4])),
            ("EndsWith", Method(&[1, 2, 3])),
            ("Equals", Method(&[1, 2])),
            ("Equals", Static(&[2, 3])),
            ("GetHashCode", Method(&[0])),
            ("GetHashCode", Static(&[1, 2])),
            ("IndexOf", Method(&[1, 2, 3, 4])),
            ("IndexOfAny", Method(&[1, 2, 3])),
            ("Insert", Method(&[2])),
            ("IsNormalized", Method(&[0, 1])),
            ("LastIndexOf", Method(&[1, 2, 3, 4])),
            ("LastIndexOfAny", Method(&[1, 2, 3])),
            ("Normalize", Method(&[0, 1])),
            ("PadLeft", Method(&[1, 2])),
            ("PadRight", Method(&[1, 2])),
            ("Remove", Method(&[1, 2])),
            ("Replace", Method(&[2, 3, 4])),
            ("Split", Method(&[1, 2, 3])),
            ("StartsWith", Method(&[1, 2, 3])),
            ("Substring", Method(&[1, 2])),
            ("ToCharArray", Method(&[0, 2])),
            ("ToLower", Method(&[0, 1])),
            ("ToLowerInvariant", Method(&[0])),
            ("ToString", Method(&[0, 1])),
            ("ToUpper", Method(&[0, 1])),
            ("ToUpperInvariant", Method(&[0])),
            ("Trim", Method(&[0, 1])),
            ("TrimEnd", Method(&[0, 1])),
            ("TrimStart", Method(&[0, 1])),
            ("get_Length", Get),
        ],
    },
    // ⚠ C# refuses a class that derives from `Array`.
    Type {
        name: "System.Array",
        value: false,
        members: &[
            ("Clone", Method(&[0])),
            ("CopyTo", Method(&[2])),
            ("GetLength", Method(&[1])),
            ("GetLongLength", Method(&[1])),
            ("GetLowerBound", Method(&[1])),
            ("GetUpperBound", Method(&[1])),
            ("GetValue", Method(&[1, 2, 3])),
            ("SetValue", Method(&[2, 3, 4])),
            ("Initialize", Method(&[0])),
            ("get_Length", Get),
            ("get_LongLength", Get),
            ("get_Rank", Get),
        ],
    },
    // ⚠ The objects of `Enum` are boxed enums, and an enum declares no method.
    Type {
        name: "System.Enum",
        value: false,
        members: &[
            ("CompareTo", Method(&[1])),
            ("HasFlag", Method(&[1])),
            ("GetTypeCode", Method(&[0])),
            ("ToString", Method(&[0, 1, 2])),
        ],
    },
    Type {
        name: "System.Random",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1])),
            ("Next", Open(&[0, 1, 2])),
            ("NextBytes", Open(&[1])),
            ("NextDouble", Open(&[0])),
            ("NextInt64", Open(&[0, 1, 2])),
            ("NextSingle", Open(&[0])),
            ("get_Shared", StaticGet),
        ],
    },
    Type {
        name: "System.WeakReference",
        value: false,
        members: &[
            ("Ctor", New(&[1, 2])),
            ("GetObjectData", Open(&[2])),
            ("get_IsAlive", OpenGet),
            ("get_Target", OpenGet),
            ("set_Target", OpenSet),
            ("get_TrackResurrection", OpenGet),
        ],
    },
    Type {
        name: "System.WeakReference_1",
        value: false,
        members: &[
            ("Ctor", New(&[1, 2])),
            ("SetTarget", Method(&[1])),
            ("TryGetTarget", Method(&[1])),
        ],
    },
    Type {
        name: "System.EventArgs",
        value: false,
        members: &[("Ctor", New(&[0]))],
    },
    Type {
        name: "System.Exception",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1, 2])),
            ("GetBaseException", Open(&[0])),
            ("ToString", Open(&[0])),
            ("get_Data", OpenGet),
            ("get_HResult", Get),
            ("get_InnerException", Get),
            ("get_Message", OpenGet),
            ("get_Source", OpenGet),
            ("get_StackTrace", OpenGet),
        ],
    },
    Type {
        name: "System.Nullable_1",
        value: true,
        members: &[
            ("GetValueOrDefault", Method(&[0, 1])),
            ("get_HasValue", Get),
            ("get_Value", Get),
        ],
    },
    Type {
        name: "System.DateTime",
        value: true,
        members: &[
            ("AddDays", Method(&[1])),
            ("AddHours", Method(&[1])),
            ("AddMilliseconds", Method(&[1])),
            ("AddMinutes", Method(&[1])),
            ("AddSeconds", Method(&[1])),
            ("AddTicks", Method(&[1])),
            ("get_Date", Get),
            ("get_Day", Get),
            ("get_DayOfWeek", Get),
            ("get_DayOfYear", Get),
            ("get_Hour", Get),
            ("get_Millisecond", Get),
            ("get_Minute", Get),
            ("get_Month", Get),
            ("get_Second", Get),
            ("get_Ticks", Get),
            ("get_TimeOfDay", Get),
            ("get_Year", Get),
            ("get_Now", StaticGet),
            ("get_Today", StaticGet),
            ("get_UtcNow", StaticGet),
        ],
    },
    Type {
        name: "System.TimeSpan",
        value: true,
        members: &[
            ("get_Days", Get),
            ("get_Hours", Get),
            ("get_Milliseconds", Get),
            ("get_Minutes", Get),
            ("get_Seconds", Get),
            ("get_Ticks", Get),
            ("get_TotalDays", Get),
            ("get_TotalHours", Get),
            ("get_TotalMilliseconds", Get),
            ("get_TotalMinutes", Get),
            ("get_TotalSeconds", Get),
        ],
    },
    Type {
        name: "System.Collections.DictionaryEntry",
        value: true,
        members: &[
            ("Ctor", New(&[2])),
            ("get_Key", Get),
            ("set_Key", Set),
            ("get_Value", Get),
            ("set_Value", Set),
        ],
    },
    Type {
        name: "System.Collections.Generic.Dictionary_2",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1, 2])),
            ("Add", Method(&[2])),
            ("Clear", Method(&[0])),
            ("ContainsKey", Method(&[1])),
            ("ContainsValue", Method(&[1])),
            ("EnsureCapacity", Method(&[1])),
            ("GetEnumerator", Method(&[0])),
            ("Remove", Method(&[1, 2])),
            ("TrimExcess", Method(&[0, 1])),
            ("TryAdd", Method(&[2])),
            ("TryGetValue", Method(&[2])),
            ("get_Comparer", Get),
            ("get_Count", Get),
            ("get_Keys", Get),
            ("get_Values", Get),
        ],
    },
    Type {
        name: "System.Collections.Generic.List_1",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1])),
            ("Add", Method(&[1])),
            ("AddRange", Method(&[1])),
            ("AsReadOnly", Method(&[0])),
            ("BinarySearch", Method(&[1, 2, 4])),
            ("Clear", Method(&[0])),
            ("Contains", Method(&[1])),
            ("ConvertAll", Method(&[1])),
            ("CopyTo", Method(&[1, 2, 4])),
            ("EnsureCapacity", Method(&[1])),
            ("Exists", Method(&[1])),
            ("Find", Method(&[1])),
            ("FindAll", Method(&[1])),
            ("FindIndex", Method(&[1, 2, 3])),
            ("FindLast", Method(&[1])),
            ("FindLastIndex", Method(&[1, 2, 3])),
            ("ForEach", Method(&[1])),
            ("GetEnumerator", Method(&[0])),
            ("GetRange", Method(&[2])),
            ("IndexOf", Method(&[1, 2, 3])),
            ("Insert", Method(&[2])),
            ("InsertRange", Method(&[2])),
            ("LastIndexOf", Method(&[1, 2, 3])),
            ("Remove", Method(&[1])),
            ("RemoveAll", Method(&[1])),
            ("RemoveAt", Method(&[1])),
            ("RemoveRange", Method(&[2])),
            ("Reverse", Method(&[0, 2])),
            ("Sort", Method(&[0, 1, 3])),
            ("ToArray", Method(&[0])),
            ("TrimExcess", Method(&[0])),
            ("TrueForAll", Method(&[1])),
            ("get_Capacity", Get),
            ("set_Capacity", Set),
            ("get_Count", Get),
        ],
    },
    Type {
        name: "System.Collections.Generic.HashSet_1",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1, 2])),
            ("Add", Method(&[1])),
            ("Clear", Method(&[0])),
            ("Contains", Method(&[1])),
            ("CopyTo", Method(&[1, 2, 3])),
            ("ExceptWith", Method(&[1])),
            ("GetEnumerator", Method(&[0])),
            ("IntersectWith", Method(&[1])),
            ("IsSubsetOf", Method(&[1])),
            ("IsSupersetOf", Method(&[1])),
            ("Overlaps", Method(&[1])),
            ("Remove", Method(&[1])),
            ("RemoveWhere", Method(&[1])),
            ("SetEquals", Method(&[1])),
            ("SymmetricExceptWith", Method(&[1])),
            ("TrimExcess", Method(&[0])),
            ("TryGetValue", Method(&[2])),
            ("UnionWith", Method(&[1])),
            ("get_Comparer", Get),
            ("get_Count", Get),
        ],
    },
    Type {
        name: "System.Collections.Generic.Queue_1",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1])),
            ("Clear", Method(&[0])),
            ("Contains", Method(&[1])),
            ("CopyTo", Method(&[2])),
            ("Dequeue", Method(&[0])),
            ("Enqueue", Method(&[1])),
            ("GetEnumerator", Method(&[0])),
            ("Peek", Method(&[0])),
            ("ToArray", Method(&[0])),
            ("TrimExcess", Method(&[0])),
            ("TryDequeue", Method(&[1])),
            ("TryPeek", Method(&[1])),
            ("get_Count", Get),
        ],
    },
    Type {
        name: "System.Collections.Generic.Stack_1",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1])),
            ("Clear", Method(&[0])),
            ("Contains", Method(&[1])),
            ("CopyTo", Method(&[2])),
            ("GetEnumerator", Method(&[0])),
            ("Peek", Method(&[0])),
            ("Pop", Method(&[0])),
            ("Push", Method(&[1])),
            ("ToArray", Method(&[0])),
            ("TrimExcess", Method(&[0])),
            ("TryPeek", Method(&[1])),
            ("TryPop", Method(&[1])),
            ("get_Count", Get),
        ],
    },
    Type {
        name: "System.Text.StringBuilder",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1, 2, 4])),
            ("Append", Method(&[1, 2, 3])),
            ("AppendFormat", Method(&[2, 3, 4, 5])),
            ("AppendLine", Method(&[0, 1])),
            ("Clear", Method(&[0])),
            ("CopyTo", Method(&[3, 4])),
            ("EnsureCapacity", Method(&[1])),
            ("Insert", Method(&[2, 3, 4])),
            ("Remove", Method(&[2])),
            ("Replace", Method(&[2, 4])),
            ("ToString", Method(&[0, 2])),
            ("get_Capacity", Get),
            ("set_Capacity", Set),
            ("get_Length", Get),
            ("set_Length", Set),
            ("get_MaxCapacity", Get),
        ],
    },
    Type {
        name: "System.Threading.CancellationToken",
        value: true,
        members: &[
            ("Ctor", New(&[1])),
            ("Register", Method(&[1, 2, 3])),
            ("ThrowIfCancellationRequested", Method(&[0])),
            ("UnsafeRegister", Method(&[2])),
            ("get_CanBeCanceled", Get),
            ("get_IsCancellationRequested", Get),
            ("get_WaitHandle", Get),
            ("get_None", StaticGet),
        ],
    },
    Type {
        name: "System.Threading.CancellationTokenSource",
        value: false,
        members: &[
            ("Ctor", New(&[0, 1, 2])),
            ("Cancel", Method(&[0, 1])),
            ("CancelAfter", Method(&[1])),
            ("CancelAsync", Method(&[0])),
            ("Dispose", Method(&[0])),
            ("TryReset", Method(&[0])),
            ("get_IsCancellationRequested", Get),
            ("get_Token", Get),
            ("CreateLinkedTokenSource", Static(&[1, 2])),
        ],
    },
    Type {
        name: "System.Threading.Tasks.Task",
        value: false,
        members: &[
            ("ConfigureAwait", Method(&[1])),
            ("ContinueWith", Method(&[1, 2, 3, 4, 5])),
            ("Dispose", Method(&[0])),
            ("GetAwaiter", Method(&[0])),
            ("RunSynchronously", Method(&[0, 1])),
            ("Start", Method(&[0, 1])),
            ("Wait", Method(&[0, 1, 2])),
            ("get_AsyncState", Get),
            ("get_CreationOptions", Get),
            ("get_Exception", Get),
            ("get_Id", Get),
            ("get_IsCanceled", Get),
            ("get_IsCompleted", Get),
            ("get_IsCompletedSuccessfully", Get),
            ("get_IsFaulted", Get),
            ("get_Status", Get),
            ("get_CompletedTask", StaticGet),
            ("get_CurrentId", StaticGet),
            ("get_Factory", StaticGet),
        ],
    },
    Type {
        name: "System.Threading.Tasks.Task_1",
        value: false,
        members: &[
            ("ConfigureAwait", Method(&[1])),
            ("ContinueWith", Method(&[1, 2, 3, 4, 5])),
            ("GetAwaiter", Method(&[0])),
            ("get_Result", Get),
        ],
    },
];
