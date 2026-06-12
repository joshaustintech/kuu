use crate::value::{
    LuaString, LuaTable, LuaFunction, LuaThread, LuaUserdata, Upvalue,
    StringId, TableId, FunctionId, ThreadId, UserdataId, UpvalueId, Value, UpvalueState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GcId {
    String(usize),
    Table(usize),
    Function(usize),
    Thread(usize),
    Userdata(usize),
    Upvalue(usize),
}

pub struct GcHeap {
    strings: Vec<Option<LuaString>>,
    tables: Vec<Option<LuaTable>>,
    functions: Vec<Option<LuaFunction>>,
    threads: Vec<Option<LuaThread>>,
    userdata: Vec<Option<LuaUserdata>>,
    upvalues: Vec<Option<Upvalue>>,

    free_strings: Vec<usize>,
    free_tables: Vec<usize>,
    free_functions: Vec<usize>,
    free_threads: Vec<usize>,
    free_userdata: Vec<usize>,
    free_upvalues: Vec<usize>,

    pub string_cache: std::collections::HashMap<Vec<u8>, StringId>,
}

impl Default for GcHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl GcHeap {
    pub fn new() -> Self {
        Self {
            strings: Vec::new(),
            tables: Vec::new(),
            functions: Vec::new(),
            threads: Vec::new(),
            userdata: Vec::new(),
            upvalues: Vec::new(),
            free_strings: Vec::new(),
            free_tables: Vec::new(),
            free_functions: Vec::new(),
            free_threads: Vec::new(),
            free_userdata: Vec::new(),
            free_upvalues: Vec::new(),
            string_cache: std::collections::HashMap::new(),
        }
    }

    // Allocation

    pub fn alloc_string(&mut self, data: Vec<u8>) -> StringId {
        if let Some(&id) = self.string_cache.get(&data) {
            return id;
        }
        let id = if let Some(idx) = self.free_strings.pop() {
            self.strings[idx] = Some(LuaString { data: data.clone() });
            StringId(idx)
        } else {
            let idx = self.strings.len();
            self.strings.push(Some(LuaString { data: data.clone() }));
            StringId(idx)
        };
        self.string_cache.insert(data, id);
        id
    }

    pub fn alloc_table(&mut self) -> TableId {
        let table = LuaTable {
            array: Vec::new(),
            hash: std::collections::HashMap::new(),
            metatable: None,
        };
        if let Some(idx) = self.free_tables.pop() {
            self.tables[idx] = Some(table);
            TableId(idx)
        } else {
            let idx = self.tables.len();
            self.tables.push(Some(table));
            TableId(idx)
        }
    }

    pub fn alloc_function(&mut self, func: LuaFunction) -> FunctionId {
        if let Some(idx) = self.free_functions.pop() {
            self.functions[idx] = Some(func);
            FunctionId(idx)
        } else {
            let idx = self.functions.len();
            self.functions.push(Some(func));
            FunctionId(idx)
        }
    }

    pub fn alloc_thread(&mut self) -> ThreadId {
        let thread = LuaThread { stack: Vec::new() };
        if let Some(idx) = self.free_threads.pop() {
            self.threads[idx] = Some(thread);
            ThreadId(idx)
        } else {
            let idx = self.threads.len();
            self.threads.push(Some(thread));
            ThreadId(idx)
        }
    }

    pub fn alloc_userdata(&mut self, data: Vec<u8>) -> UserdataId {
        let ud = LuaUserdata { data, metatable: None };
        if let Some(idx) = self.free_userdata.pop() {
            self.userdata[idx] = Some(ud);
            UserdataId(idx)
        } else {
            let idx = self.userdata.len();
            self.userdata.push(Some(ud));
            UserdataId(idx)
        }
    }

    pub fn alloc_upvalue(&mut self, val: Upvalue) -> UpvalueId {
        if let Some(idx) = self.free_upvalues.pop() {
            self.upvalues[idx] = Some(val);
            UpvalueId(idx)
        } else {
            let idx = self.upvalues.len();
            self.upvalues.push(Some(val));
            UpvalueId(idx)
        }
    }

    // Accessors

    pub fn get_string(&self, id: StringId) -> &LuaString {
        self.strings[id.0].as_ref().expect("invalid string id")
    }

    pub fn get_table(&self, id: TableId) -> &LuaTable {
        self.tables[id.0].as_ref().expect("invalid table id")
    }

    pub fn get_table_mut(&mut self, id: TableId) -> &mut LuaTable {
        self.tables[id.0].as_mut().expect("invalid table id")
    }

    pub fn get_function(&self, id: FunctionId) -> &LuaFunction {
        self.functions[id.0].as_ref().expect("invalid function id")
    }

    pub fn get_function_mut(&mut self, id: FunctionId) -> &mut LuaFunction {
        self.functions[id.0].as_mut().expect("invalid function id")
    }

    pub fn get_thread(&self, id: ThreadId) -> &LuaThread {
        self.threads[id.0].as_ref().expect("invalid thread id")
    }

    pub fn get_thread_mut(&mut self, id: ThreadId) -> &mut LuaThread {
        self.threads[id.0].as_mut().expect("invalid thread id")
    }

    pub fn get_userdata(&self, id: UserdataId) -> &LuaUserdata {
        self.userdata[id.0].as_ref().expect("invalid userdata id")
    }

    pub fn get_userdata_mut(&mut self, id: UserdataId) -> &mut LuaUserdata {
        self.userdata[id.0].as_mut().expect("invalid userdata id")
    }

    pub fn get_upvalue(&self, id: UpvalueId) -> &Upvalue {
        self.upvalues[id.0].as_ref().expect("invalid upvalue id")
    }

    pub fn get_upvalue_mut(&mut self, id: UpvalueId) -> &mut Upvalue {
        self.upvalues[id.0].as_mut().expect("invalid upvalue id")
    }

    // Garbage Collection

    pub fn collect_garbage(&mut self, roots: &[GcId]) {
        let mut marked_strings = vec![false; self.strings.len()];
        let mut marked_tables = vec![false; self.tables.len()];
        let mut marked_functions = vec![false; self.functions.len()];
        let mut marked_threads = vec![false; self.threads.len()];
        let mut marked_userdata = vec![false; self.userdata.len()];
        let mut marked_upvalues = vec![false; self.upvalues.len()];

        let mut queue = roots.to_vec();

        while let Some(id) = queue.pop() {
            match id {
                GcId::String(idx) => {
                    if !marked_strings[idx] {
                        marked_strings[idx] = true;
                    }
                }
                GcId::Table(idx) => {
                    if !marked_tables[idx] {
                        marked_tables[idx] = true;
                        if let Some(ref table) = self.tables[idx] {
                            if let Some(meta) = table.metatable {
                                queue.push(GcId::Table(meta.0));
                            }
                            for val in &table.array {
                                self.enqueue_value(*val, &mut queue);
                            }
                            for (k, v) in &table.hash {
                                self.enqueue_value(*k, &mut queue);
                                self.enqueue_value(*v, &mut queue);
                            }
                        }
                    }
                }
                GcId::Function(idx) => {
                    if !marked_functions[idx] {
                        marked_functions[idx] = true;
                        if let Some(ref func) = self.functions[idx] {
                            match func {
                                LuaFunction::Lua(closure) => {
                                    for up in &closure.upvalues {
                                        queue.push(GcId::Upvalue(up.0));
                                    }
                                }
                                LuaFunction::Rust(closure) => {
                                    for val in &closure.upvalues {
                                        self.enqueue_value(*val, &mut queue);
                                    }
                                }
                            }
                        }
                    }
                }
                GcId::Thread(idx) => {
                    if !marked_threads[idx] {
                        marked_threads[idx] = true;
                        if let Some(ref thread) = self.threads[idx] {
                            for val in &thread.stack {
                                self.enqueue_value(*val, &mut queue);
                            }
                        }
                    }
                }
                GcId::Userdata(idx) => {
                    if !marked_userdata[idx] {
                        marked_userdata[idx] = true;
                        if let Some(meta) = self.userdata[idx].as_ref().and_then(|ud| ud.metatable) {
                            queue.push(GcId::Table(meta.0));
                        }
                    }
                }
                GcId::Upvalue(idx) => {
                    if !marked_upvalues[idx] {
                        marked_upvalues[idx] = true;
                        if let Some(Upvalue { val: UpvalueState::Closed(val) }) = &self.upvalues[idx] {
                            self.enqueue_value(*val, &mut queue);
                        }
                    }
                }
            }
        }

        // Sweep phase

        for (i, opt_val) in self.strings.iter_mut().enumerate() {
            if opt_val.is_some() && !marked_strings[i] {
                if let Some(s) = opt_val {
                    self.string_cache.remove(&s.data);
                }
                *opt_val = None;
                self.free_strings.push(i);
            }
        }

        for (i, opt_val) in self.tables.iter_mut().enumerate() {
            if opt_val.is_some() && !marked_tables[i] {
                *opt_val = None;
                self.free_tables.push(i);
            }
        }

        for (i, opt_val) in self.functions.iter_mut().enumerate() {
            if opt_val.is_some() && !marked_functions[i] {
                *opt_val = None;
                self.free_functions.push(i);
            }
        }

        for (i, opt_val) in self.threads.iter_mut().enumerate() {
            if opt_val.is_some() && !marked_threads[i] {
                *opt_val = None;
                self.free_threads.push(i);
            }
        }

        for (i, opt_val) in self.userdata.iter_mut().enumerate() {
            if opt_val.is_some() && !marked_userdata[i] {
                *opt_val = None;
                self.free_userdata.push(i);
            }
        }

        for (i, opt_val) in self.upvalues.iter_mut().enumerate() {
            if opt_val.is_some() && !marked_upvalues[i] {
                *opt_val = None;
                self.free_upvalues.push(i);
            }
        }
    }

    fn enqueue_value(&self, val: Value, queue: &mut Vec<GcId>) {
        match val {
            Value::String(id) => queue.push(GcId::String(id.0)),
            Value::Table(id) => queue.push(GcId::Table(id.0)),
            Value::Function(id) => queue.push(GcId::Function(id.0)),
            Value::Thread(id) => queue.push(GcId::Thread(id.0)),
            Value::Userdata(id) => queue.push(GcId::Userdata(id.0)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_basic_collection() {
        let mut gc = GcHeap::new();

        let s1 = gc.alloc_string(b"hello".to_vec());
        let t1 = gc.alloc_table();

        // Put the string in the table
        gc.get_table_mut(t1).array.push(Value::String(s1));

        // GC with no roots should collect both
        gc.collect_garbage(&[]);
        assert!(gc.strings[s1.0].is_none());
        assert!(gc.tables[t1.0].is_none());
    }

    #[test]
    fn test_gc_retention() {
        let mut gc = GcHeap::new();

        let s1 = gc.alloc_string(b"retained".to_vec());
        let t1 = gc.alloc_table();

        gc.get_table_mut(t1).array.push(Value::String(s1));

        // GC with table as root should retain both
        gc.collect_garbage(&[GcId::Table(t1.0)]);
        assert!(gc.strings[s1.0].is_some());
        assert!(gc.tables[t1.0].is_some());
    }

    #[test]
    fn test_gc_cycle_collection() {
        let mut gc = GcHeap::new();

        let t1 = gc.alloc_table();
        let t2 = gc.alloc_table();

        // Create a cycle: t1 -> t2 -> t1
        gc.get_table_mut(t1).array.push(Value::Table(t2));
        gc.get_table_mut(t2).array.push(Value::Table(t1));

        // GC with no roots should collect both despite the cycle
        gc.collect_garbage(&[]);
        assert!(gc.tables[t1.0].is_none());
        assert!(gc.tables[t2.0].is_none());
    }
}

