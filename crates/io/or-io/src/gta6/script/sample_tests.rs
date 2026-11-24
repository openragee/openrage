//! A listing written by hand for the tests: every statement and expression form a lifted listing
//! holds, under names and hashes that are no game's.

pub(super) const LISTING: &str = r#"// import 1: program_0x0A0B0C0D, types [2, 100)
// import 2: program_0x1A2B3C4D, types [100, 250)
using Example.Runtime;
using static Example.Runtime.Vm;
#nullable enable
#pragma warning disable CS0162, CS0164, CS0219
namespace Example.Decompiled;
[VmProgram(0x11223344U)]
public static class Program_11223344
{
    private static VmValues GuidEmpty => VmValues.Zero(2);
    [VmFunction(0x000000)]
    public static void func_000000(VmValue arg0, VmValue arg1) // 0x000000
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[4];
        ref VmValue p0 = ref parameters[0];
        ref VmValue p1 = ref parameters[1];
        ref VmValue local_0 = ref locals[0];
        ref VmValue local_1 = ref locals[1];
        ref VmValue local_2 = ref locals[2];
        ref VmValue local_3 = ref locals[3];
        VmValue stack_000040_0 = default;
        VmValue stack_000040_1 = default;
        VmValue v_000010_0 = default;
        VmValues v_000018_1 = null!;
        VmValue v_000020_2 = default;
        VmValues v_000028_3 = null!;
        if (((as_u64(as_u64(Vm.Globals(0x0A0B0C0DU)[0x1230])) == 0)))
        {
            v_000010_0 = Vm.NewObject(new VmType("type_0123456789ABCDEF", 16, 77U, 0x00000100U));
            Vm.Globals(0x0A0B0C0DU)[0x1230] = as_u64(v_000010_0);
            local_0 = v_000010_0;
            if (((as_u32(load_u8(load_u64(p1, 0x8), 0x10)) == 0)))
            {
                Virtual.type_00000000000000AA.Method_3(load_u64(local_0, 0x8), local_1);
                (stack_000040_0, stack_000040_1) = (local_0, p1);
            }
            else
            {
                (stack_000040_0, stack_000040_1) = (v_000010_0, local_1);
            }
        }
        else
        {
            (stack_000040_0, stack_000040_1) = (as_u64(Vm.Globals(0x0A0B0C0DU, true)[0x10]), 0);
        }
        v_000018_1 = Imports.shared.func_0000A0(p1);
        local_1 = v_000018_1[0];
        local_2 = v_000018_1[1];
        v_000028_3 = zero_slots(2);
        store_i32(Vm.Address(v_000028_3, 0), 0x0, 0x1234ABCD);
        v_000020_2 = Virtual.type_00000000000000AA.Method_7(stack_000040_0, v_000028_3[0]);
        store_ref(v_000020_2, 0x8, stack_000040_1);
        store_u8(p0, 0x10, 1);
        store_u16(local_0, 0x98, as_u16(local_1));
        store_u64(p1, 0x8, ((as_u64(load_u64(p1, 0x8)) | as_u64(2))));
        store_bytes(local_0, 0x58, 16, new VmValues(local_1, local_2));
        store_f32_slots(p0, 0, new VmValues(p1, local_1, local_2));
        zero_bytes(Vm.Address(locals, 3), 4);
        increment_i32(p0, 12);
        decrement_i32(local_3, 16);
        Native.DEMO.DO_THING(0); // RVA: 0x01020304
        Native._0x00000000000000BB(p0, Vm.BuiltinGlobal(0)[0]); // RVA: 0x01020308
        Builtins.Example.Thing.Ctor(v_000020_2, local_0, Vm.Function(0x11223344U, 0x000100));
        Imports.module_0x00000000000000CC.func_0000B0(Vm.BuiltinFunction(2, 1));
        Vm.Intrinsic("example_barrier", 0, 0, 0, 4, 0);
        Vm.Intrinsic("example_gate", 0);
        Operations.example_set__4_0(local_0, 0, v_000018_1[0], v_000018_1[1]);
        func_000100(Vm.Address(parameters, 0));
        return;
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    [VmFunction(0x000100)]
    public static VmValue func_000100(VmValue arg0) // 0x000100
    {
        VmValue[] parameters = new VmValue[] { arg0 };
        VmValue[] locals = new VmValue[3];
        ref VmValue p0 = ref parameters[0];
        ref VmValue local_0 = ref locals[0];
        ref VmValue local_1 = ref locals[1];
        ref VmValue local_2 = ref locals[2];
        VmValue v_000104_0 = default;
        VmValues v_00010C_1 = null!;
        VmValue v_000110_2 = default;
        VmValues v_000114_3 = null!;
        VmValues v_000118_4 = null!;
        label_000100: ;
        v_000104_0 = Operations.example_cast_type_00000000000000DD_(load_u64(byte_offset(p0, 0x8), 0));
        local_0 = Operations.NewArray_type_00000000000000EE(9);
        local_1 = ((as_i32(load_i32(p0, 0x10)) + as_i32(((1UL << unchecked((int)(10)))))));
        local_2 = as_i8(p0);
        local_2 = as_u8(p0);
        local_2 = as_i16(p0);
        local_2 = as_u16(p0);
        local_2 = as_u32(p0);
        local_2 = as_i64(p0);
        local_2 = as_f64(0xBFE0000000000000UL);
        local_2 = load_i8(p0, 0xB);
        local_2 = load_i16(p0, 0x28);
        local_2 = load_u16(v_000104_0, 0x8);
        local_2 = (((as_f32(local_1) * as_f32(1.5f)) / as_f32(2f)));
        local_2 = ((as_f32(as_i32(local_1)) % as_f32(p0)));
        local_2 = ((as_u32(p0) & as_u32(((~as_u32(local_0))))));
        local_2 = ((as_u32(p0) ^ as_u32((-2))));
        local_2 = ((as_i64(p0) >> unchecked((int)(as_i64(5)))));
        local_2 = ((as_i32(p0) - as_i32((-1))));
        local_2 = (!((as_f32(p0) <= as_f32(0x3F000000))));
        local_2 = (((as_i32(p0) >= as_i32(local_0))));
        local_2 = ((as_i64(p0) != as_i64(local_1)));
        local_2 = ((as_u64(p0) > as_u64(0)));
        local_2 = ((as_i32(p0) < 7));
        local_2 = float_bits(0xFFFFFFFF);
        local_2 = 1E-45f;
        local_2 = 2.5E+10f;
        local_2 = (-0.5f);
        local_2 = 0xFFFFFFFF80000001;
        local_2 = 4000000000U;
        local_2 = true;
        local_2 = GuidEmpty[1];
        v_00010C_1 = vector3((-12.5f), 0f, 0.9f);
        v_000114_3 = extract_bytes(new VmValues(p0, local_1), 4, 8);
        v_000118_4 = load_bytes(Vm.GlobalAddress(0x0A0B0C0DU, 0x40), 0x0, 8);
        local_2 = Vm.StringConstant(0x0A0B0C0DU, 0x8U);
        local_2 = Vm.Intrinsic("example_address", 1, 1)[0];
        local_2 = Vm.TypedIntrinsic("example_box", new VmType("Example.Type", 16, 0x80000001U, 0x00000005U), p0);
        local_2 = Vm.NewObject(new VmType("bare_type"));
        local_2 = Vm.VirtualFunction(p0, new VmType("type_00000000000000AA", 1, 22U, 0x00000003U), 4);
        local_2 = Vm.GlobalAddress(0x0A0B0C0DU, 0x10, true);
        local_2 = (((((as_i32(p0) == as_i32(5))) != Truth(0)) ? (VmValue)(0x2468ACE0) : (VmValue)(7)));
        local_1++;
        local_0--;
        v_000110_2 = p0++;
        if ((!(((as_i32(local_1) < as_i32(Operations.example_length(local_0)))))))
        {
        goto label_000180;
        }
        else
        {
        goto label_000100;
        }
        label_000180: ;
        return ((as_f32(local_1) / as_f32(local_2)));

        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    [VmFunction(0x000200)]
    public static VmValues func_000200() // 0x000200
    {
        VmValue[] parameters = new VmValue[] {  };
        VmValue[] locals = new VmValue[2];
        ref VmValue local_0 = ref locals[0];
        ref VmValue local_1 = ref locals[1];
        VmValue v_000204_0 = default;
        local_0 = 0;
        while (Truth(true))
        {
            if ((!(((as_i32(local_0) < 10)))))
            {
                break;
            }
            v_000204_0 = Native.DEMO.GET_THING(local_0); // RVA: 0x0102030C
            switch (as_i32(((as_i32(v_000204_0) - 1))))
            {
                case 0:
                case 2:
                    local_1 = 1;
                    break;
                case 1:
                    goto label_000280;
                default:
                    local_0++;
                    continue;
            }
            local_0++;
        }
        label_000280: ;
        return new VmValues(local_0, local_1);
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    [VmFunction(0x000300)]
    public static void func_000300(VmValue arg0) // 0x000300
    {
        // INCOMPLETE: Widths are underdetermined.
        throw new global::System.NotSupportedException("func_000300: Widths are underdetermined.");
    }

    [VmFunction(0x000400)]
    public static void main() // 0x000400
    {
        VmValue[] parameters = new VmValue[] {  };
        VmValue[] locals = new VmValue[0];
        return;
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Builtins
    {
        public static class Example
        {
            public static class Thing
            {
                public static void Ctor(VmValue p0, VmValue p1, VmValue p2) => Vm.Builtin(2, 0, 0, p0, p1, p2);
            }
        }
    }
    public static class Imports
    {
        public static class module_0x00000000000000CC
        {
            public static void func_0000B0(VmValue p0) => Vm.Script(0x1A2B3C4DU, 0x0000B0, 0, p0);
        }
        public static class shared
        {
            public static VmValues func_0000A0(VmValue p0) => Vm.Script(0x0A0B0C0DU, 0x0000A0, 2, p0);
        }
    }
    public static class Native
    {
        public static class DEMO
        {
            public static void DO_THING(VmValue p0) => Vm.Native(0x1111222233334444UL, 0x01020304U, 0, p0);
            public static VmValue GET_THING(VmValue p0) => Vm.Native(0x5555666677778888UL, 0x0102030CU, 1, p0)[0];
        }
        public static void _0x00000000000000BB(VmValue p0, VmValue p1) => Vm.Native(0x00000000000000BBUL, 0x01020308U, 0, p0, p1);
    }
    public static class Operations
    {
        public static VmValue NewArray_type_00000000000000EE(VmValue p0) => Vm.NewArray(new VmType("type_00000000000000EE", 8, 21U, 0x00000102U), p0);
        public static VmValue example_length(VmValue p0) => Vm.Intrinsic("example_length", 1, p0)[0];
        public static void example_set__4_0(VmValue p0, VmValue p1, VmValue p2, VmValue p3) => Vm.Intrinsic("example_set", 0, p0, p1, p2, p3);
        public static VmValue example_cast_type_00000000000000DD_(VmValue p0) => Vm.Intrinsic("example_cast\u003Ctype_00000000000000DD\u003E", 1, p0)[0];
    }
    public static class Virtual
    {
        public static class type_00000000000000AA
        {
            public static void Method_3(VmValue p0, VmValue p1) => Vm.Virtual("type_00000000000000AA", 3, 0, p0, p1);
            public static VmValue Method_7(VmValue p0, VmValue p1) => Vm.Virtual("type_00000000000000AA", 7, 1, p0, p1)[0];
        }
    }
}
"#;

/// The simple statements each function of [`LISTING`] reads as, counting `v = p0++;` as two.
pub(super) const STATEMENTS: [usize; 5] = [30, 46, 5, 0, 0];
