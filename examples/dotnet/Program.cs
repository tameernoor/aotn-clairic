using System.Runtime.InteropServices;

// dotnet run -- ../../emails/z03-angrerett.eml      flag a dispute
// dotnet run -- ../../invoices/f01-faktura.jpg      read an invoice photo
if (args.Length == 0)
{
    Console.WriteLine("usage: dotnet run -- <email.eml | invoice.jpg>");
    return;
}
var raw = File.ReadAllBytes(args[0]);
var ptr = args[0].EndsWith(".eml")
    ? Native.clairic_flag_dispute(raw, (nuint)raw.Length)
    : Native.clairic_read_invoice(raw, (nuint)raw.Length);
try
{
    Console.WriteLine(Marshal.PtrToStringUTF8(ptr));
}
finally
{
    Native.clairic_free(ptr);
}

static class Native
{
    [DllImport("clairic")]
    public static extern IntPtr clairic_flag_dispute(byte[] raw, nuint len);

    [DllImport("clairic")]
    public static extern IntPtr clairic_read_invoice(byte[] raw, nuint len);

    [DllImport("clairic")]
    public static extern void clairic_free(IntPtr s);
}
