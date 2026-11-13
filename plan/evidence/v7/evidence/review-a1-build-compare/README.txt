aapt2 dump xmltree --file AndroidManifest.xml, line numbers stripped; diff apk2 vs apk-a1:
== full
identical
== combo
26,29d25
<           E: service 
<             A: http://schemas.android.com/apk/res/android:name(0x01010003)="com.example.fgslab.ExportService" (Raw: "com.example.fgslab.ExportService")
<             A: http://schemas.android.com/apk/res/android:exported(0x01010010)=false
<             A: http://schemas.android.com/apk/res/android:foregroundServiceType(0x01010599)=0x00002001
38a35,38
>           E: service 
>             A: http://schemas.android.com/apk/res/android:name(0x01010003)="com.example.fgslab.ExportService" (Raw: "com.example.fgslab.ExportService")
>             A: http://schemas.android.com/apk/res/android:exported(0x01010010)=false
>             A: http://schemas.android.com/apk/res/android:foregroundServiceType(0x01010599)=0x00002001
== none
identical
== noperm
22,25d21
<           E: service 
<             A: http://schemas.android.com/apk/res/android:name(0x01010003)="com.example.fgslab.ExportService" (Raw: "com.example.fgslab.ExportService")
<             A: http://schemas.android.com/apk/res/android:exported(0x01010010)=false
<             A: http://schemas.android.com/apk/res/android:foregroundServiceType(0x01010599)=0x40002001
34a31,34
>           E: service 
>             A: http://schemas.android.com/apk/res/android:name(0x01010003)="com.example.fgslab.ExportService" (Raw: "com.example.fgslab.ExportService")
>             A: http://schemas.android.com/apk/res/android:exported(0x01010010)=false
>             A: http://schemas.android.com/apk/res/android:foregroundServiceType(0x01010599)=0x40002001

dexdump -d of com.example.fgslab classes (apk/ vs apk2/ combo), addresses and pool indices stripped: see apk-vs-apk2-combo-appclasses.diff (only onTimeout(I)V added)

cmp of the two combo binary manifests (apk/ vs apk2/):
identical
