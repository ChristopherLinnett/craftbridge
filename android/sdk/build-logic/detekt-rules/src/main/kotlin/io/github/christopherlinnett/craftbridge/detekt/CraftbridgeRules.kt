package io.github.christopherlinnett.craftbridge.detekt

// The gate's Kotlin checks detekt does not ship, read from Kotlin's own syntax tree (plan D-28).
// The rules share their header — the same detekt api they build a rule from — so they are classes in
// this one file: split over a file each, that header would be the same lines over and over, which is
// the duplication AGENTS.md §5 refuses. The profiles in detekt-rules/profiles name which run where.

import dev.detekt.api.Config
import dev.detekt.api.Detektion
import dev.detekt.api.Entity
import dev.detekt.api.FileProcessListener
import dev.detekt.api.Finding
import dev.detekt.api.Issue
import dev.detekt.api.Location
import dev.detekt.api.RequiresAnalysisApi
import dev.detekt.api.Rule
import dev.detekt.api.RuleSetId
import dev.detekt.api.SetupContext
import dev.detekt.api.config
import dev.detekt.core.suppressors.isSuppressedBy
import org.jetbrains.kotlin.analysis.api.analyze
import org.jetbrains.kotlin.analysis.api.components.KaDiagnosticCheckerFilter
import org.jetbrains.kotlin.analysis.api.components.evaluate
import org.jetbrains.kotlin.analysis.api.components.returnType
import org.jetbrains.kotlin.analysis.api.fir.diagnostics.KaFirDiagnostic
import org.jetbrains.kotlin.analysis.api.symbols.KaCallableSymbol
import org.jetbrains.kotlin.analysis.api.symbols.KaClassSymbol
import org.jetbrains.kotlin.analysis.api.symbols.KaPropertySymbol
import org.jetbrains.kotlin.analysis.api.symbols.KaSymbolVisibility
import org.jetbrains.kotlin.analysis.api.types.symbol
import org.jetbrains.kotlin.descriptors.annotations.AnnotationUseSiteTarget
import org.jetbrains.kotlin.lexer.KtTokens
import org.jetbrains.kotlin.name.ClassId
import org.jetbrains.kotlin.name.StandardClassIds
import org.jetbrains.kotlin.psi.KtAnnotated
import org.jetbrains.kotlin.psi.KtAnnotationEntry
import org.jetbrains.kotlin.psi.KtCallableDeclaration
import org.jetbrains.kotlin.psi.KtClassBody
import org.jetbrains.kotlin.psi.KtClassOrObject
import org.jetbrains.kotlin.psi.KtDeclaration
import org.jetbrains.kotlin.psi.KtFile
import org.jetbrains.kotlin.psi.KtNamedFunction
import org.jetbrains.kotlin.psi.KtPostfixExpression
import org.jetbrains.kotlin.psi.KtProperty
import org.jetbrains.kotlin.psi.KtPropertyAccessor
import org.jetbrains.kotlin.psi.psiUtil.getStrictParentOfType
import org.jetbrains.kotlin.psi.psiUtil.isAncestor
import java.nio.file.Path
import java.util.concurrent.ConcurrentLinkedQueue

/**
 * `!!` throws a NullPointerException where library code fails with a documented exception instead
 * (AGENTS.md §3). The ban is on the operator itself, read from the syntax tree, so it holds whatever
 * the operand's type: detekt's UnsafeCallOnNullableType reports `!!` only on a type marked nullable,
 * which leaves out a Java value of platform type (`System.getProperty("k")!!`) — the very place a
 * null arrives unannounced — and any operand whose type the analysis could not settle.
 */
class NotNullAssertion(
    config: Config,
) : Rule(config, "`!!` throws a NullPointerException: library code fails with a documented exception instead") {
    override fun visitPostfixExpression(expression: KtPostfixExpression) {
        super.visitPostfixExpression(expression)
        if (expression.operationToken == KtTokens.EXCLEXCL) {
            report(
                Finding(
                    Entity.from(expression.operationReference),
                    "`!!` throws a NullPointerException: fail with a documented exception instead (AGENTS.md §3)",
                ),
            )
        }
    }
}

/**
 * A Kotlin `external` declaration is a JNI binding: every call through it crosses into native code no
 * compiler can see, so the declarations live only under a `platform/` folder — in every source set,
 * the test sets included (AGENTS.md §4). Tests take the never-crash shortcuts, never this: the native
 * surface is the one thing a reviewer has to be able to find in one place, and a test is exactly where
 * an unreviewed binding gets written first.
 */
class ExternalOutsidePlatformFolder(
    config: Config,
) : Rule(config, "external (JNI) declarations live only under a platform/ folder, in every source set") {
    // Read when a file is judged, as the package rule reads its packages: the convention plugin writes
    // the module's folder into the config each run is given, and a run without it is wired wrong.
    private val moduleDirectory: String by config("")

    // Every kind of declaration the modifier can sit on — a function, a property, an accessor alone
    // (`val version: Int external get`) — reaches the tree's one visit for declarations.
    override fun visitDeclaration(declaration: KtDeclaration) {
        super.visitDeclaration(declaration)
        if (!declaration.hasModifier(KtTokens.EXTERNAL_KEYWORD)) {
            return
        }
        val module =
            moduleDirectory.ifEmpty {
                error("ExternalOutsidePlatformFolder has no moduleDirectory: craftbridge.android.library writes the module's folder")
            }
        if (!underPlatformFolder(module, declaration.containingKtFile.virtualFilePath)) {
            report(
                Finding(
                    Entity.from(declaration),
                    "`external` (JNI) here, outside a `platform/` folder: native code the compiler cannot see " +
                        "stays in one place a reviewer can find (AGENTS.md §4)",
                ),
            )
        }
    }
}

/**
 * Does `virtualFilePath` sit under a `platform` folder of its source set? A module's sources sit at
 * `src/<source set>/<kotlin or java>/` below the module's folder — AGP's layout, which no module build
 * file can change — so the path is read from below `moduleDirectory`: the source root is the third
 * folder there, and `platform` counts only between it and the file. Nothing above the module counts, a
 * `platform`, `src`, `kotlin` or `java` folder somewhere above the checkout included, and a package below
 * the root named `kotlin` or `java` is only a package. A file outside that shape is outside `platform/`.
 */
internal fun underPlatformFolder(
    moduleDirectory: String,
    virtualFilePath: String,
): Boolean {
    val parts =
        virtualFilePath.removePrefix("${moduleDirectory.trimEnd('/')}/").takeIf { it != virtualFilePath }?.split('/') ?: return false
    val inSourceRoot = parts.size > 3 && parts[0] == "src" && (parts[2] == "kotlin" || parts[2] == "java")
    return inSourceRoot && parts.subList(3, parts.lastIndex).contains("platform")
}

/**
 * Every file of a module states the module's package — `io.github.christopherlinnett.craftbridge.<module>`
 * for the code written by hand, `io.github.christopherlinnett.craftbridge.ffi.<module>` for the generated
 * bindings (AGENTS.md §9) — so an artifact never ships a class that points at a package some other
 * module, or nothing, wrote.
 *
 * The expected packages are not this rule's to know: the convention plugin writes the module's own
 * namespace, or its bindings' spelling of it, into the config each run is given, so the rule repeats
 * nothing and a module cannot change what it is checked against from a file it is being checked on.
 * A file that declares no package is the same violation in a louder form: there is nothing to check.
 */
class ModulePackageMismatch(
    config: Config,
) : Rule(config, "every file declares its module's package") {
    // Read when a file is judged, never when the rule is built: detekt builds every rule of a rule set
    // with an empty config to learn its name, and a run that reaches a file with no list is wired to a
    // broken config, so it fails loudly rather than passing every file.
    private val expectedPackages: List<String> by config(emptyList())

    override fun visitKtFile(file: KtFile) {
        // No call to super on purpose: the package belongs to the file itself; its contents say nothing.
        val expected =
            expectedPackages.ifEmpty {
                error("ModulePackageMismatch has no expectedPackages: craftbridge.android.library writes them from the module's namespace")
            }
        // A file with no package directive reads one with an empty name, and an empty name is the finding.
        val declared = file.packageDirective?.qualifiedName
        val message =
            when {
                declared.isNullOrEmpty() -> {
                    "the file declares no package: every Kotlin file states its module's package (AGENTS.md §9)"
                }

                expected.none { declared == it || declared.startsWith("$it.") } -> {
                    "`$declared` is not this module's package: expected ${expected.joinToString(" or ")} (AGENTS.md §9)"
                }

                else -> {
                    return
                }
            }
        report(Finding(Entity.atPackageOrFirstDecl(file), message))
    }
}

/** The annotations Kotlin reads as a suppression: its own `Suppress` and Java's `SuppressWarnings`, however qualified. */
private val suppressionAnnotations = setOf("Suppress", "SuppressWarnings", "kotlin.Suppress", "java.lang.SuppressWarnings")

private fun KtAnnotationEntry.isSuppression() = typeReference?.text?.replace("`", "") in suppressionAnnotations

/** The file's suppressions detekt itself reads by name, the only ones that can keep a rule off a file. */
private fun KtFile.fileWideSuppressions() = annotationEntries.filter { it.isSuppression() }

private fun fileWideMessage(annotation: KtAnnotationEntry) =
    "`${annotation.text}` switches checks off on the whole file: switch the one item off, with a comment saying why (AGENTS.md §10)"

/** The classes the compiler reads as a suppression, whatever name a file gives them. */
private val suppressionClasses = setOf(ClassId.fromString("kotlin/Suppress"), ClassId.fromString("java/lang/SuppressWarnings"))

/**
 * The class an annotation resolves to, as the compiler answers: the name it is written with says nothing,
 * since an import alias or a typealias points at another class entirely.
 */
private fun KtAnnotationEntry.resolvedClass(): ClassId? = analyze(this) { typeReference?.type?.symbol?.classId }

private fun KtAnnotationEntry.resolvesToSuppression() = resolvedClass() in suppressionClasses

/**
 * A suppression written over a whole file switches checks off on every line of it — the compiler's
 * warnings, ktlint's rules, Lint's checks and detekt's — which is what AGENTS.md §10 refuses: a check
 * comes off on the one item that needs it, with a comment saying why, or not at all. `@file:Suppress`
 * is that switch whatever it names, so its presence is itself the finding, however it is spelled: as
 * `Suppress`, as the Java spelling `SuppressWarnings` Kotlin reads too, fully qualified, or under an
 * import alias or a typealias, which the compiler honours as the class it resolves to — so the class
 * an annotation resolves to is asked as well as the name it is written with.
 *
 * detekt runs no rule at all over a file whose own `@file:Suppress` names that rule, its rule set or
 * `all` — the very switch this rule exists to catch — so [FileWideSuppressionBackstop] reports the
 * files this rule is not let read.
 */
class FileWideSuppression(
    config: Config,
) : Rule(config, "a suppression written over a whole file switches every check off on every line of it"),
    RequiresAnalysisApi {
    override fun visitKtFile(file: KtFile) {
        // No call to super on purpose: the file's header is all this rule reads; its contents are the
        // business of the other rules, each of which the engine visits with its own visitor.
        for (annotation in file.annotationEntries.filter { it.isSuppression() || it.resolvesToSuppression() }) {
            report(Finding(Entity.from(annotation), fileWideMessage(annotation)))
        }
    }
}

/**
 * [FileWideSuppression] for the files detekt does not let it read: those whose own `@file:Suppress`
 * silences it, which detekt decides before any rule runs, by the same check asked here. A processor
 * sees every file, suppressed or not, so it files the rule's findings for those files under the rule's
 * own name, where the run counts them like any other; on every other file the rule reports for itself,
 * so nothing is reported twice. In a run that does not switch the rule on, it adds nothing.
 */
class FileWideSuppressionBackstop : FileProcessListener {
    override val id: String = "FileWideSuppressionBackstop"

    private val ruleId = FileWideSuppression::class.java.simpleName
    private val ruleSet = RuleSetId(CraftbridgeRuleSetProvider.RULE_SET)
    private lateinit var basePath: Path
    private lateinit var aliases: Set<String>
    private val unread = ConcurrentLinkedQueue<Pair<KtAnnotationEntry, Location>>()

    override fun init(context: SetupContext) {
        basePath = context.basePath
        aliases =
            context.config
                .subConfig(ruleSet.value)
                .subConfig(ruleId)
                .valueOrDefault(Config.ALIASES_KEY, emptyList<String>())
                .toSet()
    }

    override fun onProcess(file: KtFile) {
        if (file.isSuppressedBy(ruleId, aliases, ruleSet)) {
            file.fileWideSuppressions().forEach { unread += it to Location.from(it) }
        }
    }

    override fun onFinish(
        files: List<KtFile>,
        result: Detektion,
    ): Detektion {
        val rule = result.rules.firstOrNull { it.id == ruleId && it.ruleSetId == ruleSet && it.active } ?: return result
        val issues =
            unread.map { (annotation, at) ->
                Issue(
                    rule,
                    Issue.Entity(
                        Entity.from(annotation).signature,
                        Issue.Location(at.source, at.endSource, at.text, basePath.relativize(at.path)),
                    ),
                    emptyList(),
                    fileWideMessage(annotation),
                    rule.severity,
                    emptyList(),
                )
            }
        return Detektion(result.issues + issues, result.rules, result.notifications, result.metrics, result.userData)
    }
}

/**
 * A suppression of one of the compiler's own errors, which leaves the error unreported and the code it
 * covers compiled under rules the compiler calls unspecified. The compiler warns on each one, and under
 * warnings as errors that warning is the build's refusal of it, but the convention plugin turns the
 * warning off for every compilation: a compiler option reaches the whole compilation, and the generated
 * bindings must suppress the two errors explicit API mode reports on the helpers UniFFI writes with no
 * visibility. So the refusal is made here instead, on every run but the bindings' (written.yml), and
 * their folder alone is exempt.
 *
 * Which suppression covers an error is the compiler's own answer, not a list kept here: the analysis
 * reports each as the warning the compilation no longer prints, for every error the suppression names
 * however its annotation is spelt, and each such warning is a finding.
 */
class ErrorSuppression(
    config: Config,
) : Rule(config, "a suppression of a compiler error compiles code the compiler has refused"),
    RequiresAnalysisApi {
    override fun visitKtFile(file: KtFile) {
        // No call to super on purpose: the compiler's analysis of the file answers for all of it at once.
        val suppressed =
            analyze(file) {
                file
                    .collectDiagnostics(KaDiagnosticCheckerFilter.ONLY_COMMON_CHECKERS)
                    .filterIsInstance<KaFirDiagnostic.ErrorSuppression>()
                    .map { it.psi to it.diagnosticName }
            }
        for ((argument, error) in suppressed) {
            report(
                Finding(
                    Entity.from(argument),
                    "this suppresses the compiler's error $error: fix the code instead; only the generated bindings may (AGENTS.md §10)",
                ),
            )
        }
    }
}

/**
 * A suppression on one declaration that silences a rule this run enforces. Every rule detekt runs in
 * the gate is a gate rule — the never-crash bans of AGENTS.md §3, which §10 never lets an item switch
 * off, and the layout rules of §4 and §9, which have no false positive to excuse — so a suppression
 * detekt would honour for any of them is refused, while one naming only the compiler's, ktlint's or
 * Lint's checks, which §10 allows on one item with its reason, passes.
 *
 * Which annotation silences which rule is detekt's own question, so it is asked of detekt's own
 * suppression check with each active rule's id, rule set and aliases — read from this run's config,
 * so no list of rules is written twice. A finding is filed against the file, not the annotated
 * declaration: filed there, the very suppression it reports would hide it.
 */
class GateRuleSuppression(
    config: Config,
) : Rule(config, "a suppression on a declaration switches off a rule the gate enforces") {
    // The run's config holds rule sets of rules, beside settings that are plain values; only a section
    // is descended into.
    private val activeRules: List<ActiveRule> by lazy {
        val root = config.parent?.parent ?: return@lazy emptyList()
        root.sections().flatMap { (ruleSet, ruleSetConfig) ->
            ruleSetConfig
                .sections()
                .filter { (_, ruleConfig) -> ruleConfig.valueOrDefault(Config.ACTIVE_KEY, false) }
                .map { (rule, ruleConfig) ->
                    ActiveRule(rule, RuleSetId(ruleSet), ruleConfig.valueOrDefault(Config.ALIASES_KEY, emptyList<String>()).toSet())
                }
        }
    }

    override fun visitAnnotationEntry(annotationEntry: KtAnnotationEntry) {
        super.visitAnnotationEntry(annotationEntry)
        val owner = annotationEntry.getStrictParentOfType<KtAnnotated>() ?: return
        // A file's own suppression is FileWideSuppression's finding, whatever it names. Of an item's
        // annotations only its first suppression is read, so an item reports once: detekt reads all of
        // them together, and the message names every rule they silence.
        if (owner is KtFile || !annotationEntry.isSuppression() ||
            owner.annotationEntries.firstOrNull { it.isSuppression() } != annotationEntry
        ) {
            return
        }
        val enclosing = owner.getStrictParentOfType<KtAnnotated>()
        // A rule an enclosing item already silences is that item's finding, reported there.
        val silenced =
            activeRules.filter { rule ->
                owner.isSuppressedBy(rule.id, rule.aliases, rule.ruleSet) &&
                    enclosing?.isSuppressedBy(rule.id, rule.aliases, rule.ruleSet) != true
            }
        if (silenced.isNotEmpty()) {
            val file = annotationEntry.containingKtFile
            report(
                Finding(
                    Entity(Entity.from(annotationEntry).signature, Location.from(annotationEntry), file),
                    "this suppression switches off ${silenced.joinToString { it.id }}, which the gate enforces on every item: " +
                        "fix the code instead (AGENTS.md §3, §10)",
                ),
            )
        }
    }

    private class ActiveRule(
        val id: String,
        val ruleSet: RuleSetId,
        val aliases: Set<String>,
    )
}

/**
 * A public call written by hand never takes the name of a public call that generated code declares on the
 * same type (AGENTS.md §9): neither a command namespace's word on the type it hangs off, nor a command's
 * name on a namespace's type. Kotlin itself refuses one form of the clash alone, an extension property
 * that a member property of the same name shadows; a member function beside a namespace's property
 * (`fun script(…)` on `EffectsEngine`), or an extension beside a command method with other parameters
 * (`fun TemplatesCommands.list()`), compiles, and a call reaches whichever the compiler ranks first, so
 * `engine.templates.open(…)` would stop being the one call its name says it is.
 *
 * Which calls a type has, its members and the extensions of it its package declares, is the compiler's own
 * answer, however the type is spelt; what is generated is told by where a declaration sits, the module's
 * generated folder, which the convention plugin writes into the config each run is given. A call the
 * module keeps to itself (`internal`, `private`) is nobody else's to reach, so only public ones count.
 */
class GeneratedNameTaken internal constructor(
    config: Config,
    private val generatedPath: ((String) -> Boolean)?,
) : Rule(config, "a public call written by hand takes no name of a public call generated on the same type"),
    RequiresAnalysisApi {
    constructor(config: Config) : this(config, null)

    // Read when a file is judged, as the package rule reads its packages: a run without it is wired wrong.
    private val generatedFolder: String by config("")

    override fun visitNamedFunction(function: KtNamedFunction) {
        super.visitNamedFunction(function)
        check(function)
    }

    override fun visitProperty(property: KtProperty) {
        super.visitProperty(property)
        check(property)
    }

    private fun isGenerated(path: String): Boolean =
        generatedPath?.invoke(path) ?: underFolder(
            generatedFolder.ifEmpty {
                error("GeneratedNameTaken has no generatedFolder: craftbridge.android.library writes the module's generated folder")
            },
            path,
        )

    private fun check(declaration: KtCallableDeclaration) {
        val name = declaration.nameAsName ?: return
        if (isGenerated(declaration.containingKtFile.virtualFilePath)) {
            return
        }
        val taken =
            analyze(declaration) {
                val written = declaration.symbol as? KaCallableSymbol
                // The type the call is reached on: an extension's receiver, a member's class.
                val owner =
                    when {
                        written == null || written.visibility != KaSymbolVisibility.PUBLIC -> null
                        written.isExtension -> written.receiverParameter?.returnType?.symbol
                        else -> written.containingDeclaration
                    } as? KaClassSymbol
                val ownerId = owner?.classId
                if (owner == null || ownerId == null) {
                    emptyList()
                } else {
                    val extensions =
                        findTopLevelCallables(ownerId.packageFqName, name).filter { candidate ->
                            candidate.isExtension && (candidate.receiverParameter?.returnType?.symbol as? KaClassSymbol)?.classId == ownerId
                        }
                    (owner.declaredMemberScope.callables(name) + extensions)
                        .filter { generated ->
                            generated.visibility == KaSymbolVisibility.PUBLIC &&
                                generated.psi
                                    ?.containingFile
                                    ?.virtualFile
                                    ?.path
                                    ?.let(::isGenerated) == true
                        }.map { ownerId.asFqNameString() }
                        .toList()
                }
            }
        taken.firstOrNull()?.let { type ->
            report(
                Finding(
                    Entity.from(declaration),
                    "`$name` is a public call the generated facade already gives `$type`: a call written by hand takes a " +
                        "name of its own, or a call that belongs to a namespace extends that namespace's type (AGENTS.md §9)",
                ),
            )
        }
    }
}

/** Does `path` sit inside `folder`? A sibling whose name only starts the same is outside it. */
internal fun underFolder(
    folder: String,
    path: String,
): Boolean = path.startsWith("${folder.trimEnd('/')}/")

/**
 * TestNG's own annotations: kotlin-test's `Test` and `Ignore` are typealiases of the two here, and an
 * annotation resolves to the class its typealias expands to.
 */
private const val TESTNG_ANNOTATIONS = "org.testng.annotations"

private val testNgTest = ClassId.fromString("org/testng/annotations/Test")

private val testNgIgnore = ClassId.fromString("org/testng/annotations/Ignore")

private val jvmField = ClassId.fromString("kotlin/jvm/JvmField")

private fun KtAnnotated.annotation(of: ClassId): KtAnnotationEntry? = annotationEntries.firstOrNull { it.resolvedClass() == of }

/** The class whose body declares a member, the class TestNG reads it from; none for a local or top-level declaration. */
private val KtDeclaration.memberOf: KtClassOrObject? get() = (parent as? KtClassBody)?.parent as? KtClassOrObject

/**
 * An annotation written on the declaration itself; on a property, through a use-site target or on an
 * accessor, either of which lands it on an accessor, so a property carrying a `@Test` on either is a test
 * of its own, whichever accessor ([ownTests] reads each apart).
 */
private fun KtDeclaration.ownAnnotation(of: ClassId): KtAnnotationEntry? =
    annotation(of) ?: (this as? KtProperty)?.accessors?.firstNotNullOfOrNull { it.annotation(of) }

/**
 * Which of a property's two methods TestNG reads an annotation of its own on — for TestNG knows no property,
 * only the methods Kotlin compiles. An annotation written with a `set` use-site target, or on the setter,
 * is read on the setter's method; any other is read on the getter's, Kotlin putting TestNG's annotations
 * nowhere else on a property.
 */
private fun KtProperty.readOnSetter(entry: KtAnnotationEntry): Boolean =
    entry.useSiteTarget?.getAnnotationUseSiteTarget() == AnnotationUseSiteTarget.PROPERTY_SETTER ||
        entry.getStrictParentOfType<KtPropertyAccessor>()?.isSetter == true

/**
 * An annotation a property carries on one of its two methods: written with that accessor's use-site
 * target, or on the accessor itself. Each accessor is TestNG's own method, so what one carries says
 * nothing of the other.
 */
private fun KtProperty.ownAnnotationOn(
    of: ClassId,
    onSetter: Boolean,
): KtAnnotationEntry? =
    annotationEntries.firstOrNull { it.resolvedClass() == of && readOnSetter(it) == onSetter }
        ?: (if (onSetter) setter else getter)?.annotation(of)

/**
 * The `@Test`s a property carries itself, one for each method that carries one: the getter's, then the
 * setter's. A property may carry both, and TestNG reads each apart, so neither hides the other.
 */
private fun KtProperty.ownTests(): List<KtAnnotationEntry> = listOf(false, true).mapNotNull { ownAnnotationOn(testNgTest, onSetter = it) }

/**
 * The `@Ignore` TestNG reads for a test: the one on the very method the test's own `@Test` sits on, for a
 * property on that same accessor. A member that takes its test from the `@Test` over its class has none:
 * TestNG reads a member's `@Ignore` only beside the member's own `@Test`, and runs the member whatever an
 * `@Ignore` of its own says.
 */
private fun KtDeclaration.ignoreRead(test: KtAnnotationEntry): KtAnnotationEntry? =
    when {
        !isAncestor(test) -> null
        this is KtProperty -> ownAnnotationOn(testNgIgnore, readOnSetter(test))
        else -> annotation(testNgIgnore)
    }

/**
 * How many tests TestNG reads in a declaration, one for each method: a function's one, a property's own
 * `@Test`s, or, under the class's `@Test`, a property's getter and a `var`'s setter where that is public too.
 */
private fun KtDeclaration.testsRead(): Int {
    if (governingTest() == null) return 0
    if (this !is KtProperty) return 1
    ownTests().size.takeIf { it > 0 }?.let { return it }
    val property = this
    val setter = analyze(property) { (property.symbol as? KaPropertySymbol)?.setter?.visibility }
    return if (setter == KaSymbolVisibility.PUBLIC || setter == KaSymbolVisibility.INTERNAL) 2 else 1
}

/**
 * The `@Test` TestNG reads a declaration's method under, or none where it reads no test. A method's own
 * `@Test` makes it a test whatever its visibility, and alone decides how it runs, whatever its class says.
 * A `@Test` over a class makes a test, run as that `@Test` says, of every other method public in the class
 * file: a function or a property's getter Kotlin declares neither private nor protected, `internal` ones
 * included, since they compile to public methods. Left out are a property that compiles to no getter
 * (`const`, `@JvmField`) and a method carrying another of TestNG's annotations, which keeps it for another
 * job — a configuration method, a data provider — where a dropped test costs nothing.
 */
private fun KtDeclaration.governingTest(): KtAnnotationEntry? {
    val declaration = this
    if (declaration !is KtNamedFunction && declaration !is KtProperty) return null
    declaration.ownAnnotation(testNgTest)?.let { return it }
    val overClass = declaration.memberOf?.annotation(testNgTest) ?: return null
    val noGetter =
        declaration is KtProperty && (declaration.hasModifier(KtTokens.CONST_KEYWORD) || declaration.annotation(jvmField) != null)
    val otherJob =
        declaration.annotationEntries.any { entry ->
            entry
                .resolvedClass()
                ?.takeIf { it != testNgIgnore }
                ?.packageFqName
                ?.asString() == TESTNG_ANNOTATIONS
        }
    val visibility = analyze(declaration) { (declaration.symbol as? KaCallableSymbol)?.visibility }
    val public = visibility == KaSymbolVisibility.PUBLIC || visibility == KaSymbolVisibility.INTERNAL
    return overClass.takeIf { public && !noGetter && !otherJob }
}

/**
 * Whether the test sits in a class an `@Ignore` is written over: that class's finding counts it, where the
 * switch is written, so a finding of its own would report the same switch again below it.
 */
private fun KtDeclaration.countedOnItsClass(): Boolean = memberOf?.annotation(testNgIgnore) != null

/** An annotation argument's value as the compiler folds it, so a name cannot hide it: none where the argument is not written. */
private fun KtAnnotationEntry.constantArgument(name: String): Any? {
    val argument =
        valueArgumentList
            ?.arguments
            ?.firstOrNull { it.getArgumentName()?.asName?.asString() == name }
            ?.getArgumentExpression()
            ?: return null
    return analyze(argument) { argument.evaluate()?.value }
}

private const val UNSEEN = "says nothing in any report, not even a skip, so the rule it pins goes unchecked while the run reads green"

/**
 * A test as a finding names it: by its own `@Test` where it carries one, else as a test of the `@Test`
 * over its class.
 */
private fun KtDeclaration.named(test: KtAnnotationEntry): String {
    val written = if (this is KtProperty) "${valOrVarKeyword.text} $name" else "fun $name()"
    return when {
        !isAncestor(test) -> "`$written`, a test through the `@Test` over its class,"
        this is KtProperty && readOnSetter(test) -> "the setter test of `$written`"
        else -> "`@Test $written`"
    }
}

/**
 * The switch on a test that TestNG reads as off, whichever kind of test it is: an `@Ignore` beside the
 * test's own `@Test` ([ignoreRead]), and `enabled = false` or `invocationCount` below one on the `@Test`
 * that governs it — its own, which says so, or its class's, which it takes them from. One reading for a
 * function and a property's accessors, so each is read and named for its switch the same way.
 */
private fun switchedOff(
    declaration: KtDeclaration,
    test: KtAnnotationEntry,
): String? {
    val subject = declaration.named(test)
    val says = if (declaration.isAncestor(test)) "says" else "takes from that `@Test`"
    val invocations = test.constantArgument("invocationCount") as? Int
    return when {
        declaration.ignoreRead(test) != null -> {
            "$subject is annotated `@Ignore`: TestNG does not run it and $UNSEEN: delete the test or the annotation"
        }

        test.constantArgument("enabled") == false -> {
            "$subject $says `enabled = false`: TestNG does not run it and $UNSEEN: delete the test or the argument"
        }

        invocations != null && invocations < 1 -> {
            "$subject $says `invocationCount = $invocations`: TestNG runs it that many times, which is never, and $UNSEEN: " +
                "delete the test or the argument"
        }

        else -> {
            null
        }
    }
}

/**
 * A `@Test` TestNG never runs. TestNG builds each class's tests as it reads the class and drops there,
 * without one word to any report, not even as a skip, every test whose JVM method returns a value:
 * only a function bounded to `Unit`, and only non-null, compiles to a `void` one — a body bounded to a
 * `runBlocking { … }` call or another expression without naming `Unit` returns that value, a `Nothing`
 * one compiles to a method returning `java.lang.Void`, a `Unit?` one to one returning `kotlin.Unit`, a
 * `suspend` one to one that takes a continuation and returns `Object`, and a property's getter returns the
 * property — so every other return type, or one the analysis cannot settle, is refused rather than
 * trusted. It runs none of a test whose own `@Test` sits beside an `@Ignore` — a member the `@Test` over
 * its class governs runs whatever an `@Ignore` of its own says —, none of one saying `enabled = false`, and
 * a test saying `invocationCount` below one that many times, which is never. A property's two accessors are two
 * methods to TestNG, each with its own switch: a `@Test` read on the setter belongs to the setter method,
 * whose one parameter TestNG fails to supply or fills from a data provider, so the run fails such a method
 * or runs it and says so itself — only that method's own switch drops it into the silence, and no
 * accessor's switch drops the other's test. The `@Test` a test runs under is
 * its own, or, for a method that has none, the one over its class ([governingTest]), which makes a test of
 * every public method there; so a class whose `@Test` says `enabled = false` still runs each method that
 * has its own `@Test`, and runs none of the others. kotlin-test's `Test` and `Ignore` are typealiases of
 * TestNG's, so each is matched as the class it resolves to, as the compiler answers, and the return type
 * and every argument are asked of the compiler too. An `@Ignore` over a class drops every test the class
 * holds the same silent way, and is reported once on the class, where the switch is written, instead of
 * once per dropped method; TestNG reads it on that class alone, not on a class nested in it. Over a class
 * that carries `@Test` as well it switches that `@Test` off, and reads the switch on that class alone: a
 * class holding no test is switched to no test and drops nothing, so such a class passes, and only one
 * holding tests is refused. A `@Test` on
 * a private method runs, since TestNG reaches it reflectively. The check is one of detekt's, not of a
 * test report, because a report is exactly what such a test never produces (AGENTS.md §10 reads Kotlin
 * through detekt).
 */
class UnrunTest(
    config: Config,
) : Rule(config, "TestNG runs no `@Test` it drops as it reads the class, and it drops these in silence"),
    RequiresAnalysisApi {
    override fun visitClassOrObject(classOrObject: KtClassOrObject) {
        super.visitClassOrObject(classOrObject)
        val ignored = classOrObject.annotation(testNgIgnore) ?: return
        val tests = classOrObject.declarations.sumOf { it.testsRead() }
        // An `@Ignore` switches off the class it sits over, and no other, so a class that holds no test
        // — its own, or made of its members by a `@Test` it carries — has nothing the switch could drop
        // in silence, and an empty one carrying both annotations is switched to nothing, which is what
        // it says: only a class holding tests has tests to lose, and only then is there something unseen.
        if (tests == 0) {
            return
        }
        report(
            Finding(
                Entity.from(ignored),
                "`@Ignore` over this class runs none of the $tests tests TestNG reads in it and $UNSEEN: " +
                    "delete the annotation or the tests",
            ),
        )
    }

    override fun visitNamedFunction(function: KtNamedFunction) {
        super.visitNamedFunction(function)
        val problem = unrun(function) ?: return
        report(Finding(Entity.from(function), problem))
    }

    override fun visitProperty(property: KtProperty) {
        super.visitProperty(property)
        if (property.countedOnItsClass()) return
        // Each accessor is a method of its own to TestNG, so a property's own `@Test`s are read apart, one per
        // method; a property with none is a test of the `@Test` over its class, through its getter.
        for (test in property.ownTests().ifEmpty { listOfNotNull(property.governingTest()) }) {
            // The switch is read of a property's test as of a function's, and before its return: a test
            // switched off is never run, whatever its method returns, and is named for the switch that drops it.
            val switch = switchedOff(property, test)
            if (switch == null && property.isAncestor(test) && property.readOnSetter(test)) {
                // A `@Test` read on the setter is TestNG's test of the setter method, which takes a value TestNG
                // cannot supply unless a data provider fills it: TestNG fails such a method or runs it, and the
                // run says so itself, so this rule — which catches the silence — names nothing. Only the setter's
                // own switch, read above, could bring TestNG's silence back, and no message about the getter is
                // true of a test written on the setter.
                continue
            }
            val problem =
                switch
                    ?: (
                        "${property.named(test)} puts the test on the getter of `${property.name}`, which returns the property's value, " +
                            "so TestNG drops it and $UNSEEN: write the test as a function bounded to `Unit`, with the assertions inside it"
                    )
            report(Finding(Entity.from(property), problem))
        }
    }

    private fun unrun(function: KtNamedFunction): String? {
        val test = function.governingTest() ?: return null
        if (function.countedOnItsClass()) return null
        switchedOff(function, test)?.let { return it }
        val subject = function.named(test)
        return when {
            function.hasModifier(KtTokens.SUSPEND_KEYWORD) -> {
                "$subject is `suspend`, so it compiles to a method that takes a continuation and returns `Object`: TestNG drops it " +
                    "as it reads the class and $UNSEEN: write a plain function whose body runs in `runBlocking`, bounded to `Unit`"
            }

            else -> {
                returnProblem(function)?.let { "$subject $it" }
            }
        }
    }

    // Only `Unit`, and only non-null, compiles to a `void` method: `Nothing` compiles to one returning
    // `java.lang.Void` and `Unit?` to one returning `kotlin.Unit`, and TestNG drops a `@Test` whose method
    // returns either as silently as one returning any other value.
    private fun returnProblem(function: KtNamedFunction): String? {
        val (returnClass, nullable) = analyze(function) { function.returnType.symbol?.classId to function.returnType.isNullable }
        return when {
            returnClass == StandardClassIds.Unit && !nullable -> {
                null
            }

            returnClass == null -> {
                "has a return type the analysis cannot settle: TestNG drops any `@Test` that returns a value, so one no rule can " +
                    "vouch for is refused: name the return type `Unit`"
            }

            else -> {
                "returns `${returnClass.asString()}${if (nullable) "?" else ""}`: TestNG drops it as it reads the class and $UNSEEN: " +
                    "bound the body to `Unit`, with the assertions inside it"
            }
        }
    }
}
