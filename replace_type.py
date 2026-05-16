with open('src/table_function.rs', 'r') as f:
    content = f.read()

old = 'pub array: std::sync::Arc<zarrs::array::Array<std::sync::Arc<zarrs::storage::store::FilesystemStore>>>,'
new = 'pub array: std::sync::Arc<zarrs::array::Array<zarrs::storage::store::FilesystemStore>>,'

content = content.replace(old, new)

with open('src/table_function.rs', 'w') as f:
    f.write(content)
